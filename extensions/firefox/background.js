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

async function executeMethod(id, method, params) {
  try {
    let result;

    switch (method) {
      case "list_tabs": {
        const tabs = await browser.tabs.query({});
        result = tabs.map(summarizeTab);
        break;
      }

      case "list_windows": {
        const windows = await browser.windows.getAll({
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
        requireInteger(tabId, "tab_id");
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

        const createProperties = { url };
        if (params.window_id !== undefined) {
          requireInteger(params.window_id, "window_id");
          createProperties.windowId = params.window_id;
        }
        if (params.index !== undefined) {
          requireInteger(params.index, "index");
          createProperties.index = params.index;
        }

        const newTab = await browser.tabs.create(createProperties);
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

        const movedTabs = await browser.tabs.move(ids, {
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
    setTimeout(connect, RECONNECT_DELAY_MS);
  }
}

connect();
