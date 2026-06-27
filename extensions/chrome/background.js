// rustab - Chromium extension service worker.

importScripts("background_core.js");

createRustabBackground({
  api: chrome,
  keepalive: true,
});
