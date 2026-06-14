// rustab - shared browser-extension background implementation.
// Wrappers provide the browser API object and keepalive behavior.

(function (global) {
  "use strict";

  function createRustabBackground(options) {
    const api = options.api;
    const nativeHost = options.nativeHost || "rustab_mediator";
    const reconnectDelayMs = options.reconnectDelayMs || 2000;
    const keepalive = options.keepalive || null;

    let port = null;

    function connect() {
      if (port) return;

      try {
        port = api.runtime.connectNative(nativeHost);
      } catch (e) {
        console.error("rustab: failed to connect:", e);
        scheduleReconnect();
        return;
      }

      console.log("rustab: connected to native host");

      port.onMessage.addListener(handleMessage);

      port.onDisconnect.addListener(() => {
        const err = api.runtime.lastError?.message || "unknown reason";
        console.warn("rustab: native host disconnected:", err);
        port = null;
        scheduleReconnect();
      });
    }

    function scheduleReconnect() {
      setTimeout(connect, reconnectDelayMs);
    }

    function handleMessage(msg) {
      const { id, method, params } = msg;

      if (!method) {
        if (msg.type === "ping") {
          safeSend({ id, result: { pong: true, timestamp: Date.now() } });
        }
        return;
      }

      executeMethod(id, method, params || {}).catch((e) => {
        safeSend({ id, error: e.message || String(e) });
      });
    }

    function summarizeTab(tab) {
      return {
        id: tab.id,
        title: tab.title || "",
        url: tab.url || "",
        active: tab.active,
        window_id: tab.windowId,
        index: tab.index,
        pinned: tab.pinned || false,
      };
    }

    function summarizeWindow(window) {
      const tabs = window.tabs || [];
      const activeTab = tabs.find((tab) => tab.active);

      return {
        id: window.id,
        focused: window.focused || false,
        type: window.type || "",
        state: window.state || "",
        incognito: window.incognito || false,
        tab_count: tabs.length,
        active_tab_id: activeTab?.id ?? null,
        active_tab_title: activeTab?.title || "",
        active_tab_url: activeTab?.url || "",
      };
    }

    function requireInteger(value, name) {
      if (!Number.isInteger(value)) {
        throw new Error(`${name} must be an integer`);
      }
    }

    async function executeMethod(id, method, params) {
      try {
        let result;

        switch (method) {
          case "list_tabs": {
            const tabs = await api.tabs.query({});
            result = tabs.map(summarizeTab);
            break;
          }

          case "list_windows": {
            const windows = await api.windows.getAll({
              populate: true,
              windowTypes: ["normal"],
            });
            result = windows.map(summarizeWindow);
            break;
          }

          case "close_tabs": {
            const ids = params.tab_ids;
            if (!Array.isArray(ids) || ids.length === 0) {
              safeSend({ id, error: "tab_ids must be a non-empty array" });
              return;
            }
            ids.forEach((tabId) => requireInteger(tabId, "tab_ids entries"));
            await api.tabs.remove(ids);
            result = { ok: true, closed: ids.length };
            break;
          }

          case "activate_tab": {
            const tabId = params.tab_id;
            if (typeof tabId !== "number") {
              safeSend({ id, error: "tab_id must be a number" });
              return;
            }
            requireInteger(tabId, "tab_id");
            await api.tabs.update(tabId, { active: true });
            const tab = await api.tabs.get(tabId);
            await api.windows.update(tab.windowId, { focused: true });
            result = { ok: true };
            break;
          }

          case "open_tab": {
            const url = params.url;
            if (typeof url !== "string" || url.length === 0) {
              safeSend({ id, error: "url must be a non-empty string" });
              return;
            }

            const createProperties = { url };
            if (params.window_id !== undefined) {
              requireInteger(params.window_id, "window_id");
              createProperties.windowId = params.window_id;
            }
            if (params.index !== undefined) {
              requireInteger(params.index, "index");
              createProperties.index = params.index;
            }

            const newTab = await api.tabs.create(createProperties);
            result = summarizeTab(newTab);
            break;
          }

          case "move_tabs": {
            const ids = params.tab_ids;
            if (!Array.isArray(ids) || ids.length === 0) {
              safeSend({ id, error: "tab_ids must be a non-empty array" });
              return;
            }
            ids.forEach((tabId) => requireInteger(tabId, "tab_ids entries"));
            requireInteger(params.window_id, "window_id");

            const index = params.index ?? -1;
            requireInteger(index, "index");

            const movedTabs = await api.tabs.move(ids, {
              windowId: params.window_id,
              index,
            });
            const movedArray = Array.isArray(movedTabs) ? movedTabs : [movedTabs];
            result = {
              ok: true,
              moved: movedArray.length,
              tabs: movedArray.map(summarizeTab),
            };
            break;
          }

          default:
            safeSend({ id, error: `unknown method: ${method}` });
            return;
        }

        safeSend({ id, result });
      } catch (e) {
        safeSend({ id, error: e.message || String(e) });
      }
    }

    function safeSend(msg) {
      if (!port) {
        console.warn("rustab: cannot send, port disconnected");
        return;
      }

      try {
        port.postMessage(msg);
      } catch (e) {
        console.error("rustab: send failed:", e);
        port = null;
        scheduleReconnect();
      }
    }

    if (keepalive && api.alarms) {
      api.alarms.create(keepalive.name, {
        periodInMinutes: keepalive.periodInMinutes,
      });

      api.alarms.onAlarm.addListener((alarm) => {
        if (alarm.name === keepalive.name && !port) {
          connect();
        }
      });
    }

    api.runtime.onInstalled?.addListener(() => connect());
    api.runtime.onStartup?.addListener(() => connect());

    connect();
  }

  global.createRustabBackground = createRustabBackground;
})(globalThis);
