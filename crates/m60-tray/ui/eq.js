// Custom EQ preset editor. The backend owns the saved presets and talks to the speaker;
// this page edits one preset at a time. Plain JS, no bundler.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const NEW = "new";

let model = {
  presets: [], activeId: null, connected: false, controllable: false,
  freqs: [62, 250, 1000, 4000, 8000, 16000], minDb: -3, maxDb: 3, stepDb: 0.5, maxNameBytes: 20,
};
let selected = NEW;                       // a preset id, or NEW
let draft = { name: "", gains: flat() };  // what the sliders show
let base = clone(draft);                  // the saved version of the draft
let deleteArmedUntil = 0;

function flat() { return [0, 0, 0, 0, 0, 0]; }
function clone(d) { return { name: d.name, gains: [...d.gains] }; }
function freqLabel(hz) { return hz >= 1000 ? `${hz / 1000} kHz` : `${hz} Hz`; }
function dbLabel(db) { return db > 0 ? `+${db.toFixed(1)}` : db.toFixed(1); }
function presetById(id) { return model.presets.find((p) => p.id === id); }

function isDirty() {
  if (selected === NEW) return draft.name.trim() !== "" || draft.gains.some((g) => g !== 0);
  return draft.name !== base.name || draft.gains.some((g, i) => g !== base.gains[i]);
}

function message(text, kind = "") {
  const el = $("message");
  el.textContent = text;
  el.className = kind;
}

// ---- building the page ----------------------------------------------------------------------
function buildBands() {
  const row = $("bands");
  row.textContent = "";
  model.freqs.forEach((hz, i) => {
    const band = document.createElement("div");
    band.className = "band";
    const out = document.createElement("output");
    const wrap = document.createElement("div");
    wrap.className = "vslider";
    const input = document.createElement("input");
    Object.assign(input, { type: "range", min: model.minDb, max: model.maxDb, step: model.stepDb, value: 0 });
    input.dataset.band = String(i);
    input.setAttribute("aria-label", `${freqLabel(hz)} gain`);
    input.addEventListener("input", () => { draft.gains[i] = Number(input.value); render(); });
    wrap.addEventListener("wheel", (e) => {
      e.preventDefault();
      const step = e.deltaY < 0 ? model.stepDb : -model.stepDb;
      draft.gains[i] = Math.max(model.minDb, Math.min(model.maxDb, draft.gains[i] + step));
      render();
    }, { passive: false });
    const freq = document.createElement("span");
    freq.className = "freq";
    freq.textContent = freqLabel(hz);
    wrap.append(input);
    band.append(out, wrap, freq);
    row.append(band);
  });
}

function renderList() {
  const list = $("list");
  list.textContent = "";
  if (model.presets.length === 0 && selected !== NEW) selected = NEW;
  if (model.presets.length === 0) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = "No presets yet";
    list.append(li);
  }
  for (const p of model.presets) {
    const li = document.createElement("li");
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("aria-current", String(p.id === selected));
    const label = document.createElement("span");
    label.className = "label";
    label.textContent = p.id === selected ? draft.name || p.name : p.name;
    b.append(label);
    if (p.id === selected && isDirty()) {
      const dot = document.createElement("span");
      dot.className = "dirty-dot";
      dot.title = "Unsaved changes";
      b.append(dot);
    }
    if (p.id === model.activeId) {
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = "On speaker";
      b.append(badge);
    }
    b.addEventListener("click", () => choose(p.id));
    li.append(b);
    list.append(li);
  }
}

function render() {
  const dirty = isDirty();
  const existing = selected !== NEW;
  const conn = $("connection");
  conn.textContent = model.connected
    ? "Speaker connected"
    : model.controllable
      ? "Speaker free for other devices - applying a preset connects briefly"
      : "Speaker not connected - you can still edit and save";
  conn.classList.toggle("off", !model.controllable);

  if ($("name").value !== draft.name) $("name").value = draft.name;
  document.querySelectorAll("#bands input").forEach((input) => {
    const i = Number(input.dataset.band);
    const g = draft.gains[i];
    if (Number(input.value) !== g) input.value = String(g);
    const span = model.maxDb - model.minDb;
    const pos = ((g - model.minDb) / span) * 100;
    const zero = ((0 - model.minDb) / span) * 100;
    input.style.setProperty("--lo", `${Math.min(pos, zero)}%`);
    input.style.setProperty("--hi", `${Math.max(pos, zero)}%`);
    input.setAttribute("aria-valuetext", `${dbLabel(g)} dB`);
    input.closest(".band").querySelector("output").textContent = dbLabel(g);
  });

  $("delete").hidden = !existing || dirty;
  $("discard").hidden = !dirty;
  $("delete").textContent = Date.now() < deleteArmedUntil ? "Click again to delete" : "Delete";
  $("save").disabled = !dirty;
  $("apply").textContent = dirty ? "Save and apply" : "Apply to speaker";
  $("apply").disabled = !model.controllable || (!dirty && !existing);
  $("apply").title = model.controllable ? "" : "Connect the speaker to apply a preset";
  renderList();
}

// ---- actions --------------------------------------------------------------------------------
// Switch to another preset, refusing while there are unsaved changes.
function choose(id) {
  if (id === selected) return;
  if (isDirty()) {
    message("Save or discard your changes first.", "error");
    return;
  }
  load(id);
}

// Show preset `id` (or a blank new one) in the editor, dropping the current draft.
function load(id) {
  selected = id;
  const p = presetById(id);
  draft = p ? { name: p.name, gains: [...p.gains] } : { name: "", gains: flat() };
  base = clone(draft);
  deleteArmedUntil = 0;
  message("");
  render();
  if (id === NEW) $("name").focus();
}

function limitBytes(text, max) {
  const enc = new TextEncoder();
  let out = text;
  while (enc.encode(out).length > max) out = out.slice(0, -1);
  return out;
}

async function save() {
  try {
    const p = await invoke("save_preset", { id: selected === NEW ? null : selected, name: draft.name, gains: draft.gains });
    if (!presetById(p.id)) model.presets = [...model.presets, p];
    selected = p.id;
    draft = { name: p.name, gains: [...p.gains] };
    base = clone(draft);
    message(`Saved "${p.name}".`);
    render();
    return p;
  } catch (e) {
    message(String(e), "error");
    return null;
  }
}

async function apply() {
  const p = isDirty() ? await save() : presetById(selected);
  if (!p) return;
  try {
    await invoke("apply_preset", { id: p.id });
    message(`"${p.name}" is now on the speaker.`);
  } catch (e) {
    message(String(e), "error");
  }
}

async function remove() {
  if (Date.now() >= deleteArmedUntil) {
    deleteArmedUntil = Date.now() + 3000;
    render();
    setTimeout(render, 3100);
    return;
  }
  const p = presetById(selected);
  deleteArmedUntil = 0;
  try {
    await invoke("delete_preset", { id: selected });
    model.presets = model.presets.filter((x) => x.id !== selected);
    load(model.presets[0]?.id ?? NEW);
    message(p ? `Deleted "${p.name}".` : "Deleted.");
  } catch (e) {
    message(String(e), "error");
  }
}

$("name").addEventListener("input", () => {
  const v = limitBytes($("name").value, model.maxNameBytes);
  if (v !== $("name").value) $("name").value = v;
  draft.name = v;
  render();
});
$("new").addEventListener("click", () => choose(NEW));
$("flat").addEventListener("click", () => { draft.gains = flat(); render(); });
$("discard").addEventListener("click", () => { draft = clone(base); message(""); render(); });
$("save").addEventListener("click", save);
$("apply").addEventListener("click", apply);
$("delete").addEventListener("click", remove);

function update(dto, first = false) {
  model = dto;
  if (first) {
    buildBands();
    load(dto.activeId ?? dto.presets[0]?.id ?? NEW);
  } else if (selected !== NEW && !presetById(selected) && !isDirty()) {
    load(dto.presets[0]?.id ?? NEW); // it was deleted elsewhere
  }
  render();
}

listen("presets", (event) => update(event.payload));
invoke("get_presets").then((dto) => update(dto, true)).catch((e) => message(String(e), "error"));
buildBands();
render();
