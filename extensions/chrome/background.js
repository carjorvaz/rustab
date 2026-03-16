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
        result = tabs.map((t) => ({
          id: t.id,
          title: t.title || "",
          url: t.url || "",
          active: t.active,
          window_id: t.windowId,
        }));
        break;
      }

      case "close_tabs": {
        const ids = params.tab_ids;
        if (!Array.isArray(ids) || ids.length === 0) {
          safeSend({ id, error: "tab_ids must be a non-empty array" });
          return;
        }
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
        const newTab = await chrome.tabs.create({ url });
        result = {
          id: newTab.id,
          title: newTab.title || "",
          url: newTab.url || url,
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
