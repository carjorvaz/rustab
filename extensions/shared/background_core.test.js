"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const coreSource = fs.readFileSync(path.join(__dirname, "background_core.js"), "utf8");

function makeEvent() {
  const listeners = [];
  return {
    addListener(listener) {
      listeners.push(listener);
    },
    emit(...args) {
      for (const listener of listeners) listener(...args);
    },
    count() {
      return listeners.length;
    },
  };
}

function makePort() {
  const messages = [];
  return {
    messages,
    onMessage: makeEvent(),
    onDisconnect: makeEvent(),
    failPostMessage: false,
    postMessage(message) {
      if (this.failPostMessage) {
        throw new Error("post failed");
      }
      messages.push(JSON.parse(JSON.stringify(message)));
    },
  };
}

function makeClock() {
  let nextId = 1;
  const pending = new Map();

  return {
    setTimeout(callback, delay) {
      const id = nextId;
      nextId += 1;
      pending.set(id, { callback, delay });
      return id;
    },
    clearTimeout(id) {
      pending.delete(id);
    },
    pendingCount() {
      return pending.size;
    },
    pendingDelays() {
      return Array.from(pending.values(), (timer) => timer.delay);
    },
    fireNext() {
      const first = pending.keys().next().value;
      assert.notEqual(first, undefined, "expected a pending timer");
      const timer = pending.get(first);
      pending.delete(first);
      timer.callback();
    },
  };
}

function loadCore(overrides = {}) {
  const ports = [];
  const alarmListeners = [];
  const alarmsCreated = [];
  const clock = makeClock();
  const api = {
    runtime: {
      lastError: null,
      connectNative() {
        const port = makePort();
        ports.push(port);
        return port;
      },
      onInstalled: makeEvent(),
      onStartup: makeEvent(),
    },
    alarms: {
      create(name, options) {
        alarmsCreated.push({ name, options: JSON.parse(JSON.stringify(options)) });
      },
      onAlarm: {
        addListener(listener) {
          alarmListeners.push(listener);
        },
      },
    },
    tabs: {
      async query() {
        return [];
      },
    },
    windows: {
      async getAll() {
        return [];
      },
    },
    ...overrides.api,
  };
  const context = {
    console,
    setTimeout: clock.setTimeout,
    clearTimeout: clock.clearTimeout,
    globalThis: {},
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(coreSource, context, { filename: "background_core.js" });

  return {
    api,
    clock,
    ports,
    alarmsCreated,
    alarmListeners,
    createRustabBackground: context.createRustabBackground,
  };
}

function messages(port) {
  return JSON.parse(JSON.stringify(port.messages));
}

function state(background) {
  return JSON.parse(JSON.stringify(background.state()));
}

test("default API selection throws a clear error when no API is available", () => {
  const { createRustabBackground } = loadCore();
  assert.throws(
    () => createRustabBackground({}),
    /rustab: browser extension API is unavailable/,
  );
});

test("Chrome keepalive creates one alarm and reconnects on that alarm", () => {
  const core = loadCore();
  const background = core.createRustabBackground({ api: core.api, keepalive: true });

  assert.deepEqual(core.alarmsCreated, [
    {
      name: "rustab-keepalive",
      options: { periodInMinutes: 0.4 },
    },
  ]);
  assert.equal(core.ports.length, 1);
  assert.deepEqual(state(background), { connected: true, reconnectPending: false });

  core.ports[0].onDisconnect.emit();
  assert.deepEqual(state(background), { connected: false, reconnectPending: true });
  assert.equal(core.clock.pendingCount(), 1);

  core.alarmListeners[0]({ name: "rustab-keepalive" });
  assert.equal(core.clock.pendingCount(), 0);
  assert.equal(core.ports.length, 2);
  assert.deepEqual(state(background), { connected: true, reconnectPending: false });
});

test("invalid request paths send exactly one error response", async () => {
  const core = loadCore();
  core.createRustabBackground({ api: core.api });
  const port = core.ports[0];

  port.onMessage.emit({ id: 1, method: "close_tabs", params: { tab_ids: [] } });
  port.onMessage.emit({ id: 2, method: "open_tab", params: { url: "" } });
  port.onMessage.emit({ id: 3, method: "missing_method", params: {} });

  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(messages(port), [
    { id: 1, error: "tab_ids must be a non-empty array" },
    { id: 2, error: "url must be a non-empty string" },
    { id: 3, error: "unknown method: missing_method" },
  ]);
});

test("reconnect scheduling coalesces repeated failures and reconnects after timer fires", () => {
  const core = loadCore();
  const background = core.createRustabBackground({ api: core.api, reconnectDelayMs: 1235 });
  const firstPort = core.ports[0];

  firstPort.onDisconnect.emit();
  firstPort.onDisconnect.emit();

  assert.deepEqual(state(background), { connected: false, reconnectPending: true });
  assert.equal(core.clock.pendingCount(), 1);
  assert.deepEqual(core.clock.pendingDelays(), [1235]);

  core.clock.fireNext();

  assert.equal(core.ports.length, 2);
  assert.deepEqual(state(background), { connected: true, reconnectPending: false });
});

test("send failure coalesces to one pending reconnect timer", () => {
  const core = loadCore();
  const background = core.createRustabBackground({ api: core.api });
  const port = core.ports[0];

  port.failPostMessage = true;
  port.onMessage.emit({ id: 1, type: "ping" });
  port.onMessage.emit({ id: 2, type: "ping" });

  assert.deepEqual(state(background), { connected: false, reconnectPending: true });
  assert.equal(core.clock.pendingCount(), 1);
});

test("stale disconnect cannot replace a keepalive-reconnected port", () => {
  const core = loadCore();
  const background = core.createRustabBackground({ api: core.api, keepalive: true });
  const firstPort = core.ports[0];

  firstPort.failPostMessage = true;
  firstPort.onMessage.emit({ id: 1, type: "ping" });
  assert.deepEqual(state(background), { connected: false, reconnectPending: true });
  assert.equal(core.clock.pendingCount(), 1);

  core.alarmListeners[0]({ name: "rustab-keepalive" });
  const secondPort = core.ports[1];
  assert.deepEqual(state(background), { connected: true, reconnectPending: false });
  assert.equal(core.clock.pendingCount(), 0);

  firstPort.onDisconnect.emit();
  assert.deepEqual(state(background), { connected: true, reconnectPending: false });
  assert.equal(core.clock.pendingCount(), 0);

  secondPort.onMessage.emit({ id: 2, type: "ping" });
  assert.equal(secondPort.messages.length, 1);
  assert.equal(secondPort.messages[0].id, 2);
  assert.equal(secondPort.messages[0].result.pong, true);
  assert.equal(typeof secondPort.messages[0].result.timestamp, "number");
});
