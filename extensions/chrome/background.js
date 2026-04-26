// rustab - Chrome extension (Manifest V3 service worker)
// Bridges native messaging from rustab-mediator to Chrome tabs API.

const NATIVE_HOST = "rustab_mediator";
const KEEPALIVE_INTERVAL_MIN = 0.4; // ~24 seconds, under the 30s idle limit

let port = null;

function connect() {
  if (port) return;

  try {
    port = chrome.runtime.connectNative(NATIVE_HOST);
  } catch (e) {
    console.error("rustab: failed to connect:", e);
    scheduleReconnect();
    return;
  }

  console.log("rustab: connected to native host");

  port.onMessage.addListener(handleMessage);

  port.onDisconnect.addListener(() => {
    const err = chrome.runtime.lastError?.message || "unknown reason";
    console.warn("rustab: native host disconnected:", err);
    port = null;
    scheduleReconnect();
  });
}

function scheduleReconnect() {
  // Service worker may die before a long timeout fires, so keep it short
  setTimeout(() => connect(), 2000);
}

function summarizeTab(t) {
  return {
    id: t.id,
    title: t.title || "",
    url: t.url || "",
    active: t.active,
    window_id: t.windowId,
    index: t.index,
    pinned: t.pinned || false,
  };
}

function summarizeWindow(w) {
  const tabs = w.tabs || [];
  const activeTab = tabs.find((t) => t.active);

  return {
    id: w.id,
    focused: w.focused || false,
    type: w.type || "",
    state: w.state || "",
    incognito: w.incognito || false,
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

function handleMessage(msg) {
  const { id, method, params } = msg;

  if (!method) {
    // Not a request — likely a ping or unknown message
    if (msg.type === "ping") {
      safeSend({ id, result: { pong: true, timestamp: Date.now() } });
    }
    return;
  }

  executeMethod(id, method, params || {}).catch((e) => {
    safeSend({ id, error: e.message || String(e) });
  });
}

async function executeMethod(id, method, params) {
  try {
    let result;

    switch (method) {
      case "list_tabs": {
        const tabs = await chrome.tabs.query({});
        result = tabs.map(summarizeTab);
        break;
      }

      case "list_windows": {
        const windows = await chrome.windows.getAll({
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
        await chrome.tabs.remove(ids);
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
        await chrome.tabs.update(tabId, { active: true });
        const tab = await chrome.tabs.get(tabId);
        await chrome.windows.update(tab.windowId, { focused: true });
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

        const newTab = await chrome.tabs.create(createProperties);
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

        const movedTabs = await chrome.tabs.move(ids, {
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

// --- Keepalive ---
// MV3 service workers die after ~30s of inactivity.
// An active native messaging port keeps it alive, but we also
// use alarms as a safety net to reconnect if the port drops.

chrome.alarms.create("rustab-keepalive", {
  periodInMinutes: KEEPALIVE_INTERVAL_MIN,
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === "rustab-keepalive") {
    if (!port) {
      connect();
    }
  }
});

// Connect on extension lifecycle events
chrome.runtime.onInstalled.addListener(() => connect());
chrome.runtime.onStartup.addListener(() => connect());

// Also connect immediately (covers service worker restart)
connect();
