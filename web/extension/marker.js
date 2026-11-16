// Tells the Relix web app the helper is installed (and which version), so it doesn't ask for it.
document.documentElement.dataset.relixHelper = chrome.runtime.getManifest().version;
