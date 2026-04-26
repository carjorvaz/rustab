// rustab - Chromium extension service worker.

importScripts("background_core.js");

createRustabBackground({
  api: chrome,
  keepalive: {
    name: "rustab-keepalive",
    periodInMinutes: 0.4,
  },
});
