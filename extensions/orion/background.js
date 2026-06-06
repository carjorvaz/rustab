// rustab - Orion extension background bootstrap.
//
// Orion 1.0.x runs Chrome extensions reliably with a Manifest V2
// persistent background page. Keep this wrapper small and load it after
// background_core.js from manifest.json.

createRustabBackground({
  api: chrome,
});
