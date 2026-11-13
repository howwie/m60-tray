// Volume popup. The backend owns the truth (it talks to the speaker); this page shows it
// and sends changes. Plain JS on purpose: no bundler, nothing to install to build the app.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const slider = $("slider");
const SEND_EVERY_MS = 70;      // do not flood the speaker while dragging
const IGNORE_ECHO_MS = 600;    // after the user moves the slider, ignore the speaker's older values

let state = { connected: false, controllable: false, status: "Starting...", statusLine: "Starting...", volume: null, maxVolume: 16, detail: "" };
let dragging = false;
let lastUserInput = 0;
let sendTimer = null;
let lastSent = 0;
let pending = null;
let hovering = false;
const THUMB = 20; // px, must match the thumb width in popup.css

function render() {
  // usable: connected, or sharing the speaker with known settings (a change then connects first)
  const connected = Boolean(state.controllable) && state.volume !== null;
  const max = state.maxVolume || 16;
  slider.max = String(max);
  $("status").textContent = state.connected ? "Edifier M60" : (state.status || state.statusLine);
  $("detail").textContent = connected ? state.detail : "";
  for (const el of [slider, $("down"), $("up")]) el.disabled = !connected;
  if (connected && !dragging && Date.now() - lastUserInput > IGNORE_ECHO_MS) {
    slider.value = String(state.volume);
  }
  const value = Number(slider.value);
  const frac = value / max;
  slider.style.setProperty("--pct", `${frac * 100}%`);
  slider.setAttribute("aria-valuetext", connected ? `${value} of ${max}` : "not connected");
  const bubble = $("bubble");
  bubble.textContent = String(value);
  bubble.style.setProperty("--x", `${THUMB / 2 + frac * (slider.clientWidth - THUMB)}px`);
  const show = connected && (hovering || dragging || slider.matches(":focus-visible"));
  $("track").classList.toggle("show", show);
  renderMedia();
}

// Media works on the Bluetooth input only; the speaker reports what the source is playing.
function renderMedia() {
  const enabled = Boolean(state.mediaEnabled);
  for (const b of document.querySelectorAll("button.media")) b.disabled = !enabled;
  const title = $("np-title");
  const artist = $("np-artist");
  let text = "";
  let sub = "";
  let muted = true;
  if (!state.controllable) {
    text = "Not connected";
  } else if (!state.bluetooth) {
    text = "Media controls only work on the Bluetooth input";
  } else if (state.nowPlaying) {
    text = state.nowPlaying.title || state.nowPlaying.artist;
    sub = state.nowPlaying.title ? state.nowPlaying.artist : "";
    muted = false;
  } else {
    // the speaker cannot tell "nothing playing" from "playing without track info" (e.g. a game)
    text = "No track info";
  }
  title.textContent = text;
  title.title = text;
  title.classList.toggle("muted", muted);
  artist.textContent = sub;
  artist.title = sub;
}

for (const b of document.querySelectorAll("button.media")) {
  b.addEventListener("click", () => {
    invoke("media", { action: Number(b.dataset.action) }).catch(() => {});
  });
}

function sendVolume(value) {
  pending = value;
  if (sendTimer !== null) return; // the pending timer will send the latest value
  const wait = Math.max(0, SEND_EVERY_MS - (Date.now() - lastSent));
  sendTimer = setTimeout(() => {
    sendTimer = null;
    lastSent = Date.now();
    invoke("set_volume", { value: pending }).catch(() => {});
  }, wait);
}

function userSet(value) {
  const max = state.maxVolume || 16;
  const v = Math.max(0, Math.min(max, value));
  lastUserInput = Date.now();
  slider.value = String(v);
  render();
  sendVolume(v);
}

slider.addEventListener("input", () => userSet(Number(slider.value)));
slider.addEventListener("pointerdown", () => { dragging = true; render(); });
slider.addEventListener("pointerenter", () => { hovering = true; render(); });
slider.addEventListener("pointerleave", () => { hovering = false; render(); });
slider.addEventListener("focus", render);
slider.addEventListener("blur", render);
window.addEventListener("pointerup", () => { dragging = false; render(); });
$("down").addEventListener("click", () => userSet(Number(slider.value) - 1));
$("up").addEventListener("click", () => userSet(Number(slider.value) + 1));
document.addEventListener("wheel", (e) => {
  if (slider.disabled) return;
  e.preventDefault();
  userSet(Number(slider.value) + (e.deltaY < 0 ? 1 : -1));
}, { passive: false });
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") invoke("hide_popup").catch(() => {});
});

listen("state", (event) => { state = event.payload; render(); });
invoke("get_state").then((s) => { state = s; render(); }).catch(() => {});
render();
