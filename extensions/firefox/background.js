// rustab - Firefox extension (Manifest V2 persistent background script)
// Bridges native messaging from rustab-mediator to browser tabs API.
//
// Firefox MV2 background scripts are persistent — no keepalive needed.
// Firefox uses `browser.*` APIs (Promise-based) instead of `chrome.*`.

const NATIVE_HOST = "rustab_mediator";
const RECONNECT_DELAY_MS = 2000;

let port = null;

function connect() {
  if (port) return;

  try {
    port = browser.runtime.connectNative(NATIVE_HOST);
  } catch (e) {
    console.error("rustab: failed to connect:", e);
    setTimeout(connect, RECONNECT_DELAY_MS);
    return;
  }

  console.log("rustab: connected to native host");

  port.onMessage.addListener(handleMessage);

  port.onDisconnect.addListener(() => {
    console.warn("rustab: native host disconnected");
    port = null;
    setTimeout(connect, RECONNECT_DELAY_MS);
  });
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

async function executeMethod(id, method, params) {
  try {
    let result;

    switch (method) {
      case "list_tabs": {
        const tabs = await browser.tabs.query({});
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
        await browser.tabs.remove(ids);
        result = { ok: true, closed: ids.length };
        break;
      }

      case "activate_tab": {
        const tabId = params.tab_id;
        if (typeof tabId !== "number") {
          safeSend({ id, error: "tab_id must be a number" });
          return;
        }
        await browser.tabs.update(tabId, { active: true });
        const tab = await browser.tabs.get(tabId);
        await browser.windows.update(tab.windowId, { focused: true });
        result = { ok: true };
        break;
      }

      case "open_tab": {
        const url = params.url;
        if (typeof url !== "string" || url.length === 0) {
          safeSend({ id, error: "url must be a non-empty string" });
          return;
        }
        const newTab = await browser.tabs.create({ url });
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
    setTimeout(connect, RECONNECT_DELAY_MS);
  }
}

connect();
