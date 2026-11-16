// The Relix Web Helper (web/extension): most debrid and file hosts don't let a page read their
// videos, so without it many streams won't play in the browser (or play without sound). Its
// content script marks the page with its version (marker.js); here the page asks for it when it's
// missing.

const ZIP = 'relix-web-helper.zip';

/** 'ok' (installed), 'missing', or 'unsupported' (not Chrome or Edge: the helper is for those). */
export function helperState() {
  const chromium = !!globalThis.chrome || navigator.userAgentData?.brands?.some((b) => /Chromium/.test(b.brand));
  if (document.documentElement.dataset.relixHelper) return 'ok';
  return chromium ? 'missing' : 'unsupported';
}

const STEPS = `
  <ol>
    <li><a class="relix-get" href="${ZIP}" download>Download the Relix Web Helper</a> and unzip it.</li>
    <li>Open <code>chrome://extensions</code> (Edge: <code>edge://extensions</code>) in a new tab and turn on <b>Developer mode</b> (top right).</li>
    <li>Click <b>Load unpacked</b> and pick the unzipped folder.</li>
    <li>Come back here and reload the page.</li>
  </ol>
  <p class="relix-small">Already have an older version? Remove it first, then load the new one. The helper only adds a permission
  to the video requests this page makes; it can't read or change anything else.</p>`;

/** The install steps, over everything; `why` says what needs it. */
export function showInstallDialog(why) {
  document.getElementById('relix-dialog')?.remove();
  const d = document.createElement('div');
  d.id = 'relix-dialog';
  d.innerHTML = `
    <div class="relix-card" role="dialog" aria-modal="true" aria-labelledby="relix-dialog-title">
      <h2 id="relix-dialog-title">Install the Relix Web Helper</h2>
      <p class="relix-why"></p>
      ${helperState() === 'unsupported' ? '<p class="relix-warn">The helper works in Chrome and Edge. In this browser, open the stream in the Relix app instead.</p>' : STEPS}
      <div class="relix-actions">
        <a class="relix-primary relix-get" href="${ZIP}" download>Download the helper</a>
        <button class="relix-secondary" type="button">Close</button>
      </div>
    </div>`;
  d.querySelector('.relix-why').textContent = why;
  const close = () => d.remove();
  d.querySelector('.relix-secondary').addEventListener('click', close);
  d.addEventListener('click', (e) => e.target === d && close());
  d.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') close();
    e.stopPropagation();
  });
  document.body.append(d);
  d.querySelector('.relix-primary').focus();
}

/** On opening the app: a banner while the helper is missing (dismissed for this visit). */
export function helperBanner() {
  const state = helperState();
  if (state === 'ok' || sessionStorageGet('relix-helper-dismissed')) return;
  const b = document.createElement('div');
  b.id = 'relix-banner';
  b.innerHTML = state === 'unsupported'
    ? `<span><b>Streams may not play in this browser.</b> Most debrid hosts block web players; the Relix Web Helper
       that fixes it is for Chrome and Edge. Use one of those, or the Relix app.</span>
       <button class="relix-x" aria-label="Dismiss">✕</button>`
    : `<span><b>Install the Relix Web Helper to play streams.</b> Without it, most debrid and file hosts block the
       web player: videos won't start, or play without sound.</span>
       <button class="relix-how" type="button">How to install</button>
       <a class="relix-primary" href="${ZIP}" download>Get the helper</a>
       <button class="relix-x" aria-label="Dismiss">✕</button>`;
  b.querySelector('.relix-how')?.addEventListener('click', () => showInstallDialog('Debrid and file hosts block web players from reading their videos; the helper lets Relix play them.'));
  b.querySelector('.relix-x').addEventListener('click', () => {
    sessionStorageSet('relix-helper-dismissed', '1');
    b.remove();
  });
  document.body.append(b);
}

// Storage can be off (private windows, blocked site data): the banner just shows again.
function sessionStorageGet(k) {
  try {
    return sessionStorage.getItem(k);
  } catch {
    return null;
  }
}
function sessionStorageSet(k, v) {
  try {
    sessionStorage.setItem(k, v);
  } catch {}
}
