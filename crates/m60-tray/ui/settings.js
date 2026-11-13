// Settings window. App settings are saved by the backend; speaker settings are sent to the
// speaker (and come back as state). Plain JS, no bundler.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
let s = null;

function message(text) {
  $("message").textContent = text;
  fit();
}

// Ask for the window to be exactly as tall as the content (header + everything in main).
// Measured right away, not in requestAnimationFrame: the window starts hidden, and a hidden
// window gets no animation frames on Windows, so the request would never be sent.
let askedFor = 0;
function contentHeight() {
  const main = document.querySelector("main");
  // main's scrollHeight counts everything in it, padding and the margins of the last items
  // included. Squeeze main to nothing while measuring, so the result does not depend on the
  // window's current height, and hide its scrollbar so the text wraps as it will once it fits.
  const saved = main.style.cssText;
  main.style.flex = "none";
  main.style.height = "0px";
  main.style.overflow = "hidden";
  const inner = main.scrollHeight;
  main.style.cssText = saved;
  const top = document.querySelector("header").getBoundingClientRect().bottom;
  return Math.ceil(top + inner) + 1; // +1: never a scrollbar from rounding at odd display scales
}
function fit() {
  const height = contentHeight();
  if (!height || height === askedFor) return;
  askedFor = height;
  invoke("fit_settings", { height }).catch(() => { askedFor = 0; }); // try again next time
}
// The window's size changed: measure again. Its width matters too: if the page was first laid
// out at another width, text that wraps onto more lines at the real width makes it taller.
window.addEventListener("resize", fit);
document.fonts?.ready.then(fit);

function buildChoices(container, name, choices, onPick) {
  if (container.childElementCount) return;
  for (const c of choices) {
    const id = `${name}-${c.id}`;
    const input = Object.assign(document.createElement("input"), { type: "radio", name, id, value: String(c.id) });
    const label = Object.assign(document.createElement("label"), { htmlFor: id, textContent: c.name });
    input.addEventListener("change", () => onPick(c.id));
    container.append(input, label);
  }
}

function render() {
  if (!s) return;
  $("model").textContent = s.model || "Not connected";
  $("firmware").textContent = s.firmware || "-";
  $("app-version").textContent = s.appVersion || "-";
  $("build-time").textContent = s.buildTime || "-";

  for (const box of document.querySelectorAll("[data-app]")) box.checked = Boolean(s[box.dataset.app]);
  const resume = $("resume");
  resume.hidden = !s.resumeInput;
  resume.textContent = s.resumeInput ? `Waiting for the speaker to come back on, to switch it back to ${s.resumeInput}.` : "";

  buildChoices($("durations"), "duration", s.durations, (v) => invoke("set_backlight", { duration: v, sensitivity: null }).catch(fail));
  buildChoices($("sensitivities"), "sensitivity", s.sensitivities, (v) => invoke("set_backlight", { duration: null, sensitivity: v }).catch(fail));
  const usable = Boolean(s.controllable);
  // the speaker sends both backlight values together, so both must be known to change one
  const backlightKnown = s.backlightDuration !== null && s.backlightSensitivity !== null;
  for (const fs of document.querySelectorAll("fieldset.choice")) fs.disabled = !usable || !backlightKnown;
  for (const r of document.querySelectorAll('input[name="duration"]')) r.checked = Number(r.value) === s.backlightDuration;
  for (const r of document.querySelectorAll('input[name="sensitivity"]')) r.checked = Number(r.value) === s.backlightSensitivity;
  const tone = $("tone");
  tone.disabled = !usable || s.btPrompt === null;
  tone.checked = s.btPrompt === true;
  tone.closest(".row").classList.toggle("disabled", tone.disabled);
  $("speaker-note").hidden = usable;
  fit();
}

function fail(e) {
  message(String(e));
  invoke("get_settings").then((d) => { s = d; render(); }).catch(() => {});
}

for (const box of document.querySelectorAll("[data-app]")) {
  box.addEventListener("change", () => {
    message("");
    invoke("set_app_setting", { key: box.dataset.app, on: box.checked }).catch(fail);
  });
}
$("tone").addEventListener("change", () => invoke("set_bt_prompt", { on: $("tone").checked }).catch(fail));

listen("settings", (event) => { s = event.payload; render(); });
invoke("get_settings").then((d) => { s = d; render(); }).catch((e) => message(String(e)));
