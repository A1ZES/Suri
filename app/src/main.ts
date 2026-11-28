/// <reference types="vite/client" />
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type Settings = Record<string, unknown> & { depth: "One" | "Two"; width: number; height: number; gamma: number | null; title: string | null };
type Device = { id: string; name: string; width: number; height: number };
type ScanInfo = { title: string; chapters: number; images: number; skipped: number };
type Item = { path: string; info?: ScanInfo; state: "ready" | "working" | "done" | "error"; note: string };
type ConvertResult = { out: string; pages: number; skipped: string[] };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const STORE = "suri:settings";
const OUT_STORE = "suri:out-dir";

function remember(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* storage unavailable: not remembered */
  }
}

function recall(key: string): string {
  try {
    return localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

/** Adds (or selects) a custom option in a select, keeping the first built-in option. */
function setCustomOption(sel: HTMLSelectElement, value: string, label: string) {
  while (sel.options.length > 1) sel.remove(1);
  if (value) sel.add(new Option(label, value));
  sel.value = value || sel.options[0].value;
}

const outDir = () => $<HTMLSelectElement>("out-dir").value || null;

let settings: Settings;
let defaults: Settings;
let devices: Device[] = [];
const queue: Item[] = [];
let selected = -1;
let pageIndex = 0;
let busy = false;

// ---------- settings <-> form ----------

function loadStored(): Partial<Settings> {
  try {
    return JSON.parse(localStorage.getItem(STORE) ?? "{}");
  } catch {
    return {};
  }
}

function store() {
  try {
    localStorage.setItem(STORE, JSON.stringify(settings));
  } catch {
    /* storage unavailable: settings just aren't remembered */
  }
}

function fillForm() {
  document.querySelectorAll<HTMLInputElement | HTMLSelectElement>("[data-key]").forEach((el) => {
    const v = settings[el.dataset.key!];
    if (el instanceof HTMLInputElement && el.type === "checkbox") el.checked = Boolean(v);
    else el.value = String(v);
  });
  document.querySelectorAll<HTMLInputElement>('input[name="depth"]').forEach((r) => (r.checked = r.value === settings.depth));
  const dev = devices.find((d) => d.width === settings.width && d.height === settings.height);
  $<HTMLSelectElement>("device").value = dev?.id ?? devices[0]?.id ?? "";
  $<HTMLInputElement>("gamma-auto").checked = settings.gamma === null;
  $<HTMLInputElement>("gamma").value = String(settings.gamma ?? 1);
  $<HTMLInputElement>("gamma").disabled = settings.gamma === null;
  $("gamma-out").textContent = settings.gamma === null ? "" : String(settings.gamma);
  $<HTMLInputElement>("title").value = settings.title ?? "";
}

function changed() {
  store();
  schedulePreview();
}

function bindForm() {
  document.querySelectorAll<HTMLInputElement | HTMLSelectElement>("[data-key]").forEach((el) => {
    el.addEventListener("change", () => {
      const key = el.dataset.key!;
      if (el instanceof HTMLInputElement && el.type === "checkbox") settings[key] = el.checked;
      else if (el.dataset.type === "num") settings[key] = Number(el.value);
      else if (el.dataset.type === "bool") settings[key] = el.value === "true";
      else settings[key] = el.value;
      changed();
    });
  });
  document.querySelectorAll<HTMLInputElement>('input[name="depth"]').forEach((r) =>
    r.addEventListener("change", () => {
      settings.depth = r.value as Settings["depth"];
      // 1-bit without dithering posterizes; 2-bit looks best undithered (matches x4converter)
      settings.dither = settings.depth === "One" ? "Atkinson" : "None";
      fillForm();
      changed();
    }),
  );
  $<HTMLSelectElement>("device").addEventListener("change", (e) => {
    const d = devices.find((x) => x.id === (e.target as HTMLSelectElement).value)!;
    settings.width = d.width;
    settings.height = d.height;
    changed();
  });
  $<HTMLInputElement>("gamma-auto").addEventListener("change", (e) => {
    settings.gamma = (e.target as HTMLInputElement).checked ? null : Number($<HTMLInputElement>("gamma").value);
    fillForm();
    changed();
  });
  $<HTMLInputElement>("gamma").addEventListener("change", (e) => {
    settings.gamma = Number((e.target as HTMLInputElement).value);
    fillForm();
    changed();
  });
  $<HTMLInputElement>("title").addEventListener("change", (e) => {
    const v = (e.target as HTMLInputElement).value.trim();
    settings.title = v === "" ? null : v;
    store();
  });
}

// ---------- queue ----------

function renderQueue() {
  const ul = $("queue");
  ul.replaceChildren();
  if (queue.length === 0) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = "Drop manga folders or CBZ files here";
    ul.append(li);
    return;
  }
  queue.forEach((item, i) => {
    const li = document.createElement("li");
    li.classList.toggle("selected", i === selected);
    const name = document.createElement("span");
    name.className = "name";
    name.title = item.path;
    name.textContent = item.info?.title ?? item.path.split(/[\\/]/).pop() ?? item.path;
    const meta = document.createElement("span");
    meta.className = `meta state ${item.state}`;
    meta.textContent = item.note || (item.info ? `${item.info.chapters} ch · ${item.info.images} img` : "scanning…");
    const remove = document.createElement("button");
    remove.className = "remove";
    remove.textContent = "✕";
    remove.setAttribute("aria-label", `Remove ${name.textContent}`);
    remove.disabled = busy;
    remove.onclick = (e) => {
      e.stopPropagation();
      queue.splice(i, 1);
      if (selected >= queue.length) selected = queue.length - 1;
      renderQueue();
      schedulePreview();
    };
    li.append(name, meta, remove);
    li.onclick = () => select(i);
    ul.append(li);
  });
}

function select(i: number) {
  selected = i;
  pageIndex = 0;
  renderQueue();
  schedulePreview();
}

async function add(paths: string[]) {
  for (const path of paths) {
    if (queue.some((q) => q.path === path)) continue;
    const item: Item = { path, state: "ready", note: "" };
    queue.push(item);
    if (selected < 0) selected = queue.length - 1;
    renderQueue();
    try {
      item.info = await invoke<ScanInfo>("scan", { path });
      if (item.info.skipped) item.note = `${item.info.chapters} ch · ${item.info.images} img · ${item.info.skipped} unreadable`;
    } catch (e) {
      item.state = "error";
      item.note = String(e);
    }
    renderQueue();
    schedulePreview();
  }
}

// ---------- preview ----------

let previewTimer: number | undefined;
let previewSeq = 0;
let urls: string[] = [];

function schedulePreview() {
  clearTimeout(previewTimer);
  previewTimer = window.setTimeout(renderPreview, 250);
}

async function renderPreview() {
  const box = $("pages");
  const item = queue[selected];
  if (!item?.info) {
    $("page-label").textContent = "–";
    return;
  }
  pageIndex = Math.max(0, Math.min(pageIndex, item.info.images - 1));
  $("page-label").textContent = `image ${pageIndex + 1} / ${item.info.images}`;
  const seq = ++previewSeq;
  box.classList.add("loading");
  try {
    const pngs = await invoke<number[][]>("preview", { path: item.path, index: pageIndex, settings });
    if (seq !== previewSeq) return; // a newer preview is on its way
    urls.forEach(URL.revokeObjectURL);
    urls = pngs.map((p) => URL.createObjectURL(new Blob([new Uint8Array(p)], { type: "image/png" })));
    box.style.setProperty("--count", String(urls.length));
    box.replaceChildren(
      ...urls.map((u, i) => {
        const img = document.createElement("img");
        img.src = u;
        img.alt = `Output page ${i + 1} of ${urls.length}`;
        return img;
      }),
    );
  } catch (e) {
    if (seq === previewSeq) box.replaceChildren(Object.assign(document.createElement("p"), { className: "hint", textContent: String(e) }));
  } finally {
    if (seq === previewSeq) box.classList.remove("loading");
  }
}

// ---------- convert ----------

function setBusy(b: boolean) {
  busy = b;
  ["convert", "add-folder", "add-cbz", "clear"].forEach((id) => ($<HTMLButtonElement>(id).disabled = b));
  $<HTMLProgressElement>("progress").hidden = !b;
  renderQueue();
}

function status(text: string, error = false) {
  const el = $("status");
  el.textContent = text;
  el.classList.toggle("error", error);
}

async function convertAll() {
  const todo = queue.filter((q) => q.info && q.state !== "done");
  if (todo.length === 0) return status("Add a manga folder or CBZ first.");
  setBusy(true);
  let failed = 0;
  for (const item of todo) {
    item.state = "working";
    item.note = "converting…";
    renderQueue();
    try {
      const r = await invoke<ConvertResult>("convert", { path: item.path, settings, outDir: outDir() });
      item.state = "done";
      item.note = r.skipped.length ? `${r.pages} pages · ${r.skipped.length} skipped` : `${r.pages} pages`;
      status(`Saved ${r.out}`);
      r.skipped.forEach((s) => console.warn("skipped", s));
    } catch (e) {
      failed++;
      item.state = "error";
      item.note = String(e);
    }
    renderQueue();
  }
  setBusy(false);
  if (failed) status(`${failed} of ${todo.length} failed — see the queue.`, true);
}

// ---------- presets ----------

async function loadPreset() {
  const path = await open({ filters: [{ name: "Preset", extensions: ["toml"] }] });
  if (!path) return;
  try {
    settings = { ...defaults, ...(await invoke<Settings>("load_preset", { path })) };
    setCustomOption($<HTMLSelectElement>("preset"), "custom", path.split(/[\\/]/).pop()!.replace(/\.toml$/i, ""));
    fillForm();
    changed();
    status(`Loaded preset ${path}`);
  } catch (e) {
    status(String(e), true);
  }
}

async function savePreset() {
  const path = await save({ defaultPath: "my-preset.toml", filters: [{ name: "Preset", extensions: ["toml"] }] });
  if (!path) return;
  try {
    await invoke("save_preset", { path, settings });
    status(`Saved preset ${path}`);
  } catch (e) {
    status(String(e), true);
  }
}

// ---------- updates ----------

async function checkForUpdate() {
  // Offline, dev builds or a missing latest.json just mean "no update": never bother the user.
  const update = await check().catch(() => null);
  if (!update) return;
  if (!(await ask(`Suri ${update.version} is available. Install it now?`, { title: "Update available", okLabel: "Update", cancelLabel: "Later" }))) return;
  status(`Downloading Suri ${update.version}…`);
  try {
    await update.downloadAndInstall();
    await relaunch();
  } catch (e) {
    status(`Update failed: ${e}`, true);
  }
}

// ---------- startup ----------

window.addEventListener("DOMContentLoaded", async () => {
  [devices, defaults] = await Promise.all([invoke<Device[]>("devices"), invoke<Settings>("default_settings")]);
  settings = { ...defaults, ...loadStored() };
  $<HTMLSelectElement>("device").replaceChildren(
    ...devices.map((d) => Object.assign(document.createElement("option"), { value: d.id, textContent: `${d.name} (${d.width}×${d.height})` })),
  );
  fillForm();
  bindForm();

  $("preset").addEventListener("change", (e) => {
    if ((e.target as HTMLSelectElement).value !== "builtin") return;
    settings = { ...defaults };
    setCustomOption($<HTMLSelectElement>("preset"), "", "");
    fillForm();
    changed();
  });
  const savedOut = recall(OUT_STORE);
  setCustomOption($<HTMLSelectElement>("out-dir"), savedOut, savedOut);
  $("out-dir").addEventListener("change", () => remember(OUT_STORE, outDir() ?? ""));
  $("choose-out").onclick = async () => {
    const dir = await open({ directory: true });
    if (typeof dir !== "string") return;
    setCustomOption($<HTMLSelectElement>("out-dir"), dir, dir);
    remember(OUT_STORE, dir);
  };
  $("load-preset").onclick = loadPreset;
  $("save-preset").onclick = savePreset;
  $("add-folder").onclick = async () => {
    const p = await open({ directory: true, multiple: true });
    if (p) add(Array.isArray(p) ? p : [p]);
  };
  $("add-cbz").onclick = async () => {
    const p = await open({ multiple: true, filters: [{ name: "Comic archive", extensions: ["cbz", "zip"] }] });
    if (p) add(Array.isArray(p) ? p : [p]);
  };
  $("clear").onclick = () => {
    queue.length = 0;
    selected = -1;
    renderQueue();
    $("pages").replaceChildren(Object.assign(document.createElement("p"), { className: "hint", textContent: "Add a manga to see how its pages will look on the device." }));
    $("page-label").textContent = "–";
  };
  $("prev").onclick = () => {
    pageIndex--;
    schedulePreview();
  };
  $("next").onclick = () => {
    pageIndex++;
    schedulePreview();
  };
  $("convert").onclick = convertAll;

  await listen<{ done: number; total: number }>("progress", (e) => {
    const p = $<HTMLProgressElement>("progress");
    p.max = e.payload.total;
    p.value = e.payload.done;
    status(`Converting image ${e.payload.done} of ${e.payload.total}`);
  });
  await getCurrentWebview().onDragDropEvent((e) => {
    const q = $("queue");
    if (e.payload.type === "over" || e.payload.type === "enter") q.classList.add("dragging");
    else q.classList.remove("dragging");
    if (e.payload.type === "drop" && !busy) add(e.payload.paths);
  });
  if (!import.meta.env.DEV) checkForUpdate();
});
