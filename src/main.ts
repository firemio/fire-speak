// fire-speak — settings window (index.html)

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { applyDom, displayHotkey, getLang, LANGS, setLang, t, tMsg } from "./i18n";
import type {
  AppStatus,
  DownloadProgressPayload,
  HistoryEntry,
  LlmProvider,
  Mode,
  ProviderKind,
  Settings,
  SetupStatus,
  StatusChangedPayload,
  SttEngine,
  UpdateInfo,
  UpdateProgressPayload,
} from "./types";

/** Whisper language codes offered in the recognition-language selector
 * (SPEC v0.5). Native names need no translation. */
const STT_LANGS: readonly { code: string; label: string }[] = [
  { code: "ja", label: "日本語" },
  { code: "en", label: "English" },
  { code: "zh", label: "中文" },
  { code: "ko", label: "한국어" },
  { code: "es", label: "Español" },
  { code: "fr", label: "Français" },
  { code: "de", label: "Deutsch" },
  { code: "pt", label: "Português" },
  { code: "ru", label: "Русский" },
  { code: "vi", label: "Tiếng Việt" },
  { code: "id", label: "Bahasa Indonesia" },
];

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element #${id}`);
  return el;
}

function input(id: string): HTMLInputElement {
  return $(id) as HTMLInputElement;
}

function selectEl(id: string): HTMLSelectElement {
  return $(id) as HTMLSelectElement;
}

function errMsg(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

/** Localized error text: backend Err values are "KEY|p0|p1" message keys. */
function errText(e: unknown): string {
  return tMsg(errMsg(e));
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function toast(message: string, isError = false): void {
  const host = $("toast-host");
  const t = el("div", `toast${isError ? " err" : ""}`, message);
  host.appendChild(t);
  window.setTimeout(() => {
    t.style.opacity = "0";
    t.style.transition = "opacity 0.25s";
    window.setTimeout(() => t.remove(), 300);
  }, 2600);
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024 * 1024) return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${n} B`;
}

function formatTime(unixMs: number): string {
  const d = new Date(unixMs);
  return d.toLocaleString(getLang(), {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------

let settings: Settings | null = null;
let setupStatus: SetupStatus | null = null;
let historyEntries: HistoryEntry[] = [];

let saveTimer: number | undefined;
let apiKeyDirty = false;

/** Last 5 saved settings snapshots, used to suppress settings-changed echoes.
 * Snapshots are stored with active_mode_id stripped: the tray switches modes
 * autonomously, and a mode round-trip re-produces byte-identical JSON that a
 * naive ring would swallow forever. */
const savedSnapshots: string[] = [];

function snapshotKey(s: Settings): string {
  return JSON.stringify({ ...s, active_mode_id: "" });
}

function recordSavedSnapshot(json: string): void {
  savedSnapshots.push(json);
  if (savedSnapshots.length > 5) savedSnapshots.shift();
}

const activeDownloads = new Map<string, DownloadProgressPayload>();

function statusLabel(status: AppStatus): string {
  return t(`status.${status}`);
}

/** Last status payload, re-rendered when the UI language changes. */
let lastStatusPayload: StatusChangedPayload = { status: "idle" };

/** Update info shown in the home banner (kept for language-change re-render). */
let bannerInfo: UpdateInfo | null = null;

/** Result of the last manual/auto update check (update section). */
let updateInfo: UpdateInfo | null = null;

// ---------------------------------------------------------------------------
// persistence
// ---------------------------------------------------------------------------

function scheduleSave(): void {
  if (saveTimer !== undefined) window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    saveTimer = undefined;
    void doSave();
  }, 500);
}

/** Persist settings immediately. Throws on backend rejection (nothing is persisted then). */
async function persistSettings(): Promise<void> {
  if (!settings) return;
  if (saveTimer !== undefined) {
    window.clearTimeout(saveTimer);
    saveTimer = undefined;
  }
  apiKeyDirty = false;
  recordSavedSnapshot(snapshotKey(settings));
  await invoke("save_settings", { settings });
}

async function doSave(): Promise<void> {
  try {
    await persistSettings();
  } catch (e: unknown) {
    toast(t("err.saveFailed", errText(e)), true);
  }
}

/** API key inputs: keep in-memory, persist on blur only. */
function markApiKeyDirty(): void {
  apiKeyDirty = true;
}

function flushApiKeySave(): void {
  if (apiKeyDirty) void doSave();
}

// ---------------------------------------------------------------------------
// navigation
// ---------------------------------------------------------------------------

const SECTION_NAMES = ["home", "general", "stt", "llm", "modes", "history", "update"];

function showSection(name: string): void {
  for (const btn of document.querySelectorAll<HTMLButtonElement>(".nav-item")) {
    btn.classList.toggle("is-active", btn.dataset.section === name);
  }
  for (const sec of document.querySelectorAll<HTMLElement>(".section")) {
    sec.classList.toggle("is-active", sec.id === `section-${name}`);
  }
  // settings may have changed on another screen: refresh the status card
  if (name === "home") void refreshSetupStatus();
  // Keep the data-i18n key in sync so applyDom() re-translates the title on
  // language change.
  const title = $("section-title");
  if (SECTION_NAMES.includes(name)) {
    title.dataset.i18n = `nav.${name}`;
    title.textContent = t(`nav.${name}`);
  } else {
    delete title.dataset.i18n;
    title.textContent = name;
  }
  if (name === "home") {
    renderHome();
  } else if (name === "history") {
    void refreshHistory();
  } else if (name === "stt") {
    void refreshSetupStatus();
  }
}

function setupNav(): void {
  $("nav").addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>(".nav-item");
    if (btn?.dataset.section) showSection(btn.dataset.section);
  });
}

// ---------------------------------------------------------------------------
// status pill
// ---------------------------------------------------------------------------

function setStatusPill(payload: StatusChangedPayload): void {
  lastStatusPayload = payload;
  const pill = $("status-pill");
  pill.dataset.status = payload.status;
  const message = payload.message !== undefined ? tMsg(payload.message) : undefined;
  const label =
    payload.status === "error" && message
      ? t("status.errorWith", message)
      : message && payload.status !== "done"
        ? t("status.withMessage", statusLabel(payload.status), message)
        : statusLabel(payload.status);
  $("status-label").textContent = label;
  pill.title = message ?? "";
}

// ---------------------------------------------------------------------------
// home
// ---------------------------------------------------------------------------

function renderHome(): void {
  if (!settings) return;
  $("home-hotkey").textContent = displayHotkey(settings.hotkey);
  renderOnboard();
  renderHomeStatus();
  renderHomeModes();
  renderHomeRecent();
}

function renderHomeModes(): void {
  if (!settings) return;
  const host = $("home-modes");
  host.textContent = "";
  for (const mode of settings.modes) {
    const chip = el("button", "mode-chip-btn", mode.name);
    chip.type = "button";
    // The instruction is long; keep it as a tooltip rather than on the card.
    chip.title = mode.use_llm ? mode.instruction || t("home.llmDefault") : t("home.noLlm");
    if (mode.id === settings.active_mode_id) chip.classList.add("is-active");
    chip.addEventListener("click", () => void activateMode(mode.id));
    host.appendChild(chip);
  }
}

async function activateMode(modeId: string): Promise<void> {
  if (!settings) return;
  settings.active_mode_id = modeId;
  renderHomeModes();
  renderModeList();
  try {
    await invoke("set_active_mode", { modeId });
  } catch (e: unknown) {
    toast(t("err.modeSwitch", errText(e)), true);
  }
}

function renderHomeRecent(): void {
  const host = $("home-recent");
  host.textContent = "";
  const recent = historyEntries.slice(0, 3);
  if (recent.length === 0) {
    const empty = el("div", "empty-state");
    empty.appendChild(el("span", "empty-icon", "🎤"));
    empty.appendChild(el("span", undefined, t("home.empty")));
    host.appendChild(empty);
    return;
  }
  for (const entry of recent) {
    host.appendChild(buildHistoryItem(entry, false));
  }
}

// ---------------------------------------------------------------------------
// home: first-run setup card (v0.9.1)
// ---------------------------------------------------------------------------

/** Approximate download size of each server build (MB), for the setup total. */
const SERVER_BUILD_MB: Record<string, number> = { cuda: 643, vulkan: 20, npu: 3, cpu: 8 };
/** AMD's compiled NPU encoder; only large-v3-turbo's size is known here. */
const NPU_ENCODER_MB: Record<string, number> = { "large-v3-turbo": 708 };

let quickSetupRunning = false;

interface OnboardStep {
  key: string;
  label: string;
  sizeMb: number;
  done: boolean;
}

/** Steps of the one-click setup for the current machine, or null if the local engine is not in use. */
function onboardSteps(s: SetupStatus): OnboardStep[] | null {
  if (!settings || settings.stt.engine !== "local") return null;
  const accel = s.effective_accel;
  const customServer = settings.stt.local.server_path.trim() !== "";
  const model = s.recommended_model;
  const steps: OnboardStep[] = [
    {
      key: downloadKey("server", "whisper-server"),
      label: t("onboard.stepServer", ACCEL_SHORT[accel] ?? "CPU"),
      sizeMb: SERVER_BUILD_MB[accel] ?? 8,
      done: s.server_installed && (customServer || buildSatisfies(s.server_backend || "cpu", accel)),
    },
    {
      key: downloadKey("model", model),
      label: t("onboard.stepModel", model),
      sizeMb: s.models.find((m) => m.name === model)?.size_mb ?? 0,
      done: s.model_installed,
    },
  ];
  if (accel === "npu") {
    steps.push({
      key: downloadKey("npu", "npu-encoder"),
      label: t("onboard.stepNpu"),
      sizeMb: NPU_ENCODER_MB[model] ?? 0,
      done: s.npu_cache_ready,
    });
  }
  return steps;
}

/** Show the setup card instead of the "ready" hero until the local engine can run. */
function renderOnboard(): void {
  const card = $("home-setup");
  const hero = $("home-hero");
  const s = setupStatus;
  const steps = s ? onboardSteps(s) : null;
  const pending = steps?.filter((st) => !st.done) ?? [];
  const show = !!s && !!steps && pending.length > 0;
  card.hidden = !show;
  hero.hidden = show;
  if (!show || !s || !steps) return;

  const found = s.hw.gpus.length ? s.hw.gpus.join(" / ") : t("stt.noGpu");
  $("onboard-detected").textContent = t("onboard.detected", found, ACCEL_SHORT[s.effective_accel] ?? "CPU");

  const list = $("onboard-steps");
  list.textContent = "";
  for (const st of steps) {
    const li = el("li", `onboard-step${st.done ? " is-done" : ""}`);
    li.appendChild(el("span", "onboard-mark", st.done ? "✓" : "○"));
    const body = el("div", "onboard-body");
    const line = el("div", "onboard-line");
    line.appendChild(el("span", "onboard-label", st.label));
    if (st.sizeMb > 0) line.appendChild(el("span", "onboard-size", formatBytes(st.sizeMb * 1024 * 1024)));
    body.appendChild(line);
    if (!st.done) body.appendChild(buildProgressEl(st.key));
    li.appendChild(body);
    list.appendChild(li);
  }

  const totalMb = pending.reduce((sum, st) => sum + st.sizeMb, 0);
  const btn = $("btn-quick-setup") as HTMLButtonElement;
  btn.disabled = quickSetupRunning;
  btn.textContent = quickSetupRunning
    ? t("onboard.running")
    : t("onboard.start", formatBytes(totalMb * 1024 * 1024));
}

async function runQuickSetup(): Promise<void> {
  if (quickSetupRunning) return;
  quickSetupRunning = true;
  renderOnboard();
  try {
    await invoke("quick_setup");
    toast(t("onboard.done", settings ? displayHotkey(settings.hotkey) : ""));
  } catch (e: unknown) {
    // Every download error was already toasted by its download-progress
    // event; only the "still not ready" result comes from quick_setup itself.
    if (String(e).startsWith("ERR_SETUP_REQUIRED")) {
      toast(t("onboard.failed", errText(e)), true);
    }
  } finally {
    quickSetupRunning = false;
    await refreshSetupStatus();
  }
}

function wireOnboard(): void {
  $("btn-quick-setup").addEventListener("click", () => void runQuickSetup());
  $("btn-onboard-cloud").addEventListener("click", async () => {
    if (!settings) return;
    settings.stt.engine = "cloud";
    try {
      await persistSettings();
    } catch (e: unknown) {
      toast(errText(e), true);
    }
    renderSttSection();
    renderOnboard();
    showSection("stt");
  });
}

// ---------------------------------------------------------------------------
// home: status card — what is installed / configured (v0.9.2)
// ---------------------------------------------------------------------------

/** Mirror of `llm::is_configured`: a key, or an OpenAI-compatible server on localhost. */
function providerReady(p: LlmProvider): boolean {
  if (p.api_key.trim() !== "") return true;
  if (p.kind === "anthropic") return false;
  try {
    const host = new URL(p.base_url).hostname.replace(/^\[|\]$/g, "").toLowerCase();
    return ["localhost", "127.0.0.1", "0.0.0.0", "::1"].includes(host);
  } catch {
    return false;
  }
}

type StatusLevel = "ok" | "warn" | "missing";

function statusRow(level: StatusLevel, label: string, value: string, section: string): HTMLElement {
  const row = el("div", `sys-row is-${level}`);
  row.appendChild(el("span", "sys-mark", level === "ok" ? "✓" : level === "warn" ? "!" : "✕"));
  row.appendChild(el("span", "sys-label", label));
  row.appendChild(el("span", "sys-value", value));
  const link = el("button", "btn btn-ghost btn-sm sys-change", t("sys.change")) as HTMLButtonElement;
  link.type = "button";
  link.addEventListener("click", () => showSection(section));
  row.appendChild(link);
  return row;
}

/** Model name from a resolved model path (`…/ggml-large-v3-turbo.bin` -> `large-v3-turbo`). */
function modelNameFromPath(path: string): string {
  const file = path.split(/[\\/]/).pop() ?? path;
  return file.replace(/^ggml-/, "").replace(/\.bin$/, "");
}

function renderHomeStatus(): void {
  const card = $("home-status");
  const heading = $("home-status-heading");
  const s = setupStatus;
  // While the setup card is up it already lists what is missing.
  const hidden = !settings || !s || !$("home-setup").hidden;
  card.hidden = hidden;
  heading.hidden = hidden;
  if (hidden || !settings || !s) return;

  card.textContent = "";
  if (settings.stt.engine === "local") {
    const backend = s.server_backend || "cpu";
    const build = t("sys.build", ACCEL_SHORT[backend] ?? "CPU");
    card.appendChild(
      s.server_installed
        ? statusRow("ok", t("sys.engine"), `${build} · ${t(s.server_running ? "sys.running" : "sys.idle")}`, "stt")
        : statusRow("missing", t("sys.engine"), t("sys.notInstalled"), "stt"),
    );
    card.appendChild(
      s.model_installed
        ? statusRow("ok", t("sys.model"), modelNameFromPath(s.model_path), "stt")
        : statusRow("missing", t("sys.model"), t("sys.notInstalled"), "stt"),
    );
    const found = s.hw.gpus.length ? s.hw.gpus.join(" / ") : t("stt.noGpu");
    const mismatch = s.server_installed && !buildSatisfies(backend, s.effective_accel);
    card.appendChild(
      statusRow(
        mismatch ? "warn" : "ok",
        t("sys.device"),
        `${found} → ${ACCEL_SHORT[s.effective_accel] ?? "CPU"}`,
        "stt",
      ),
    );
    if (s.effective_accel === "npu") {
      card.appendChild(
        statusRow(
          s.npu_cache_ready ? "ok" : "missing",
          t("sys.npu"),
          t(s.npu_cache_ready ? "common.installed" : "sys.notInstalled"),
          "stt",
        ),
      );
    }
  } else {
    const cloud = settings.stt.cloud;
    let host = cloud.base_url;
    try {
      host = new URL(cloud.base_url).host;
    } catch {
      /* keep the raw value */
    }
    const hasKey = cloud.api_key.trim() !== "";
    card.appendChild(
      statusRow(
        hasKey ? "ok" : "missing",
        t("sys.cloud"),
        hasKey ? t("sys.cloudReady", `${host} / ${cloud.model}`) : t("sys.cloudKeyMissing"),
        "stt",
      ),
    );
  }

  const lang = STT_LANGS.find((l) => l.code === settings?.language);
  card.appendChild(
    lang
      ? statusRow("ok", t("sys.language"), lang.label, "general")
      : statusRow("warn", t("sys.language"), t("sys.langAuto"), "general"),
  );

  const mode = settings.modes.find((m) => m.id === settings?.active_mode_id);
  const provider = settings.llm.providers.find((p) => p.id === settings?.llm.active_provider_id);
  if (mode && !mode.use_llm) {
    card.appendChild(statusRow("ok", t("sys.llm"), t("sys.llmOff", mode.name), "llm"));
  } else if (provider && providerReady(provider)) {
    card.appendChild(statusRow("ok", t("sys.llm"), `${provider.name} (${provider.model})`, "llm"));
  } else {
    card.appendChild(statusRow("warn", t("sys.llm"), t("sys.llmNoKey", provider?.name ?? "—"), "llm"));
  }
}

// ---------------------------------------------------------------------------
// stt section
// ---------------------------------------------------------------------------

function renderSttSection(): void {
  if (!settings) return;
  const engine = settings.stt.engine;
  for (const btn of document.querySelectorAll<HTMLButtonElement>("#stt-engine-seg button")) {
    btn.classList.toggle("is-active", btn.dataset.engine === engine);
  }
  $("stt-local-panel").hidden = engine !== "local";
  $("stt-cloud-panel").hidden = engine !== "cloud";

  input("in-local-port").value = String(settings.stt.local.server_port);
  input("in-local-threads").value = String(settings.stt.local.threads);
  input("in-local-model-path").value = settings.stt.local.model_path;
  input("in-local-server-path").value = settings.stt.local.server_path;

  input("in-cloud-base-url").value = settings.stt.cloud.base_url;
  input("in-cloud-api-key").value = settings.stt.cloud.api_key;
  input("in-cloud-model").value = settings.stt.cloud.model;

  renderSttModelList();
}

/** Build the managed model path from setup_status.models_dir. */
function managedModelPathFor(name: string): string | null {
  if (!setupStatus || !setupStatus.models_dir) return null;
  const dir = setupStatus.models_dir.replace(/[\\/]+$/, "");
  return `${dir}\\ggml-${name}.bin`;
}

function renderSttModelList(): void {
  if (!settings) return;
  const host = $("stt-model-list");
  host.textContent = "";
  const models = setupStatus?.models ?? [];
  if (models.length === 0) {
    host.appendChild(el("div", "hint", t("stt.loadingModels")));
    return;
  }
  const currentPath = settings.stt.local.model_path;

  /** One row: radio option (select when installed) + optional download button + progress. */
  const addRow = (
    label: string,
    meta: string,
    selected: boolean,
    installed: boolean,
    onSelect: (() => void) | null,
    download: { name: string; installed: boolean } | null,
  ): void => {
    const wrap = el("div", "model-row-wrap");
    const row = el("div", "model-row");
    const opt = el("div", "model-option");
    if (selected) opt.classList.add("is-selected");
    if (!installed) opt.classList.add("is-disabled");
    opt.appendChild(el("span", "radio"));
    opt.appendChild(el("span", "model-option-name", label));
    opt.appendChild(el("span", "model-option-meta", meta));
    if (installed && onSelect) opt.addEventListener("click", onSelect);
    row.appendChild(opt);
    if (download) {
      const key = downloadKey("model", download.name);
      const btn = el(
        "button",
        `btn btn-sm${download.installed ? "" : " btn-primary"}`,
        download.installed ? t("setup.redownload") : t("setup.download"),
      ) as HTMLButtonElement;
      btn.type = "button";
      btn.disabled = activeDownloads.has(key);
      btn.addEventListener("click", () => void startModelDownload(download.name));
      row.appendChild(btn);
      wrap.appendChild(row);
      wrap.appendChild(buildProgressEl(key));
    } else {
      wrap.appendChild(row);
    }
    host.appendChild(wrap);
  };

  addRow(
    t("stt.autoModel"),
    setupStatus?.model_installed ? t("stt.available") : t("stt.modelNotInstalled"),
    currentPath === "",
    true,
    () => {
      if (!settings) return;
      settings.stt.local.model_path = "";
      input("in-local-model-path").value = "";
      renderSttModelList();
      scheduleSave();
    },
    null,
  );

  for (const m of models) {
    const managed = managedModelPathFor(m.name);
    const selected = managed !== null && currentPath === managed && currentPath !== "";
    addRow(
      m.name,
      m.installed
        ? t("stt.modelInstalledMeta", m.size_mb)
        : t("stt.modelNotInstalledMeta", m.size_mb),
      selected,
      m.installed && managed !== null,
      () => {
        if (!settings || managed === null) return;
        settings.stt.local.model_path = managed;
        input("in-local-model-path").value = managed;
        renderSttModelList();
        scheduleSave();
      },
      { name: m.name, installed: m.installed },
    );
  }
}

function wireSttSection(): void {
  $("stt-engine-seg").addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-engine]");
    if (!btn || !settings) return;
    settings.stt.engine = btn.dataset.engine as SttEngine;
    renderSttSection();
    scheduleSave();
  });

  input("in-local-port").addEventListener("input", () => {
    if (!settings) return;
    const v = parseInt(input("in-local-port").value, 10);
    if (Number.isFinite(v) && v > 0 && v < 65536) {
      settings.stt.local.server_port = v;
      scheduleSave();
    }
  });
  input("in-local-threads").addEventListener("input", () => {
    if (!settings) return;
    const v = parseInt(input("in-local-threads").value, 10);
    if (Number.isFinite(v) && v > 0) {
      settings.stt.local.threads = v;
      scheduleSave();
    }
  });
  input("in-local-model-path").addEventListener("input", () => {
    if (!settings) return;
    settings.stt.local.model_path = input("in-local-model-path").value.trim();
    renderSttModelList();
    scheduleSave();
  });
  input("in-local-server-path").addEventListener("input", () => {
    if (!settings) return;
    settings.stt.local.server_path = input("in-local-server-path").value.trim();
    scheduleSave();
  });

  input("in-cloud-base-url").addEventListener("input", () => {
    if (!settings) return;
    settings.stt.cloud.base_url = input("in-cloud-base-url").value.trim();
    scheduleSave();
  });
  input("in-cloud-model").addEventListener("input", () => {
    if (!settings) return;
    settings.stt.cloud.model = input("in-cloud-model").value.trim();
    scheduleSave();
  });
  // Presets fill URL + model only; the API key is provider-specific and
  // stays for the user to paste (v0.8.1).
  const STT_PRESETS: Record<string, { base_url: string; model: string }> = {
    openai: { base_url: "https://api.openai.com/v1", model: "whisper-1" },
    groq: { base_url: "https://api.groq.com/openai/v1", model: "whisper-large-v3-turbo" },
  };
  document.querySelectorAll<HTMLButtonElement>("[data-stt-preset]").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      const preset = STT_PRESETS[btn.dataset.sttPreset ?? ""];
      if (!settings || !preset) return;
      settings.stt.cloud.base_url = preset.base_url;
      settings.stt.cloud.model = preset.model;
      input("in-cloud-base-url").value = preset.base_url;
      input("in-cloud-model").value = preset.model;
      scheduleSave();
    });
  });

  const cloudKey = input("in-cloud-api-key");
  cloudKey.addEventListener("input", () => {
    if (!settings) return;
    settings.stt.cloud.api_key = cloudKey.value;
    markApiKeyDirty();
  });
  cloudKey.addEventListener("blur", flushApiKeySave);

  $("btn-cloud-key-eye").addEventListener("click", (e) => {
    e.preventDefault();
    cloudKey.type = cloudKey.type === "password" ? "text" : "password";
  });

  const testBtn = $("btn-test-stt") as HTMLButtonElement;
  testBtn.addEventListener("click", async () => {
    const result = $("stt-test-result");
    testBtn.disabled = true;
    result.className = "test-result";
    result.textContent = t("common.testing");
    try {
      await doSave();
      const msg = await invoke<string>("test_stt");
      result.classList.add("ok");
      result.textContent = `✓ ${tMsg(msg)}`;
    } catch (e: unknown) {
      result.classList.add("err");
      result.textContent = `✗ ${errText(e)}`;
    } finally {
      testBtn.disabled = false;
    }
  });
}

// ---------------------------------------------------------------------------
// llm section (provider CRUD)
// ---------------------------------------------------------------------------

function renderProviderList(): void {
  if (!settings) return;
  const host = $("provider-list");
  host.textContent = "";
  if (settings.llm.providers.length === 0) {
    const empty = el("div", "empty-state");
    empty.appendChild(el("span", "empty-icon", "✨"));
    empty.appendChild(el("span", undefined, t("llm.empty")));
    host.appendChild(empty);
    return;
  }
  for (const provider of settings.llm.providers) {
    host.appendChild(buildProviderCard(provider));
  }
}

function buildProviderCard(provider: LlmProvider): HTMLElement {
  const card = el("div", "entity-card");
  const isActive = settings?.llm.active_provider_id === provider.id;
  if (isActive) card.classList.add("is-active-entity");

  // head: active radio + name + delete
  const head = el("div", "entity-head");

  const radioLabel = el("label", "active-radio");
  const radio = el("input") as HTMLInputElement;
  radio.type = "radio";
  radio.name = "active-provider";
  radio.checked = isActive;
  radio.addEventListener("change", () => {
    if (!settings) return;
    settings.llm.active_provider_id = provider.id;
    renderProviderList();
    scheduleSave();
  });
  radioLabel.append(radio, el("span", undefined, t("common.active")));

  const nameInput = el("input", "name-input") as HTMLInputElement;
  nameInput.type = "text";
  nameInput.value = provider.name;
  nameInput.placeholder = t("llm.namePlaceholder");
  nameInput.addEventListener("input", () => {
    provider.name = nameInput.value;
    scheduleSave();
  });

  const delBtn = el("button", "icon-btn danger", "🗑");
  delBtn.title = t("common.delete");
  delBtn.addEventListener("click", () => {
    if (!settings) return;
    if (!window.confirm(t("llm.deleteConfirm", provider.name))) return;
    settings.llm.providers = settings.llm.providers.filter((p) => p.id !== provider.id);
    if (settings.llm.active_provider_id === provider.id) {
      settings.llm.active_provider_id = settings.llm.providers[0]?.id ?? "";
    }
    renderProviderList();
    scheduleSave();
  });

  head.append(radioLabel, nameInput, delBtn);

  // form grid
  const grid = el("div", "form-grid");

  const kindField = el("label", "field");
  kindField.appendChild(el("span", "field-label", t("llm.kind")));
  const kindSel = el("select") as HTMLSelectElement;
  for (const [value, label] of [
    ["anthropic", t("llm.kindAnthropic")],
    ["openai", t("llm.kindOpenai")],
  ] as [string, string][]) {
    const opt = el("option", undefined, label) as HTMLOptionElement;
    opt.value = value;
    kindSel.appendChild(opt);
  }
  kindSel.value = provider.kind;
  kindSel.addEventListener("change", () => {
    provider.kind = kindSel.value as ProviderKind;
    scheduleSave();
  });
  kindField.appendChild(kindSel);

  const modelField = el("label", "field");
  modelField.appendChild(el("span", "field-label", t("llm.model")));
  const modelInput = el("input") as HTMLInputElement;
  modelInput.type = "text";
  modelInput.spellcheck = false;
  modelInput.value = provider.model;
  modelInput.placeholder = "claude-haiku-4-5";
  modelInput.addEventListener("input", () => {
    provider.model = modelInput.value.trim();
    scheduleSave();
  });
  modelField.appendChild(modelInput);

  const urlField = el("label", "field field-wide");
  urlField.appendChild(el("span", "field-label", t("llm.baseUrl")));
  const urlInput = el("input") as HTMLInputElement;
  urlInput.type = "text";
  urlInput.spellcheck = false;
  urlInput.value = provider.base_url;
  urlInput.placeholder = "https://api.anthropic.com";
  urlInput.addEventListener("input", () => {
    provider.base_url = urlInput.value.trim();
    scheduleSave();
  });
  urlField.appendChild(urlInput);

  const keyField = el("label", "field field-wide");
  keyField.appendChild(el("span", "field-label", t("llm.apiKey")));
  const keyWrap = el("span", "input-with-btn");
  const keyInput = el("input") as HTMLInputElement;
  keyInput.type = "password";
  keyInput.spellcheck = false;
  keyInput.autocomplete = "off";
  keyInput.value = provider.api_key;
  keyInput.placeholder = "sk-...";
  keyInput.addEventListener("input", () => {
    provider.api_key = keyInput.value;
    markApiKeyDirty();
  });
  keyInput.addEventListener("blur", flushApiKeySave);
  const eyeBtn = el("button", "btn btn-ghost btn-sm eye-btn", "👁") as HTMLButtonElement;
  eyeBtn.type = "button";
  eyeBtn.title = t("common.showHide");
  eyeBtn.addEventListener("click", () => {
    keyInput.type = keyInput.type === "password" ? "text" : "password";
  });
  keyWrap.append(keyInput, eyeBtn);
  keyField.appendChild(keyWrap);

  grid.append(kindField, modelField, urlField, keyField);

  // test row
  const testRow = el("div", "test-row inline-test");
  const testBtn = el("button", "btn btn-sm", t("llm.testBtn")) as HTMLButtonElement;
  const testResult = el("span", "test-result");
  testBtn.addEventListener("click", async () => {
    testBtn.disabled = true;
    testResult.className = "test-result";
    testResult.textContent = t("common.testing");
    try {
      await doSave();
      // test_llm's Ok value is the model's raw reply — NOT a message key (SPEC).
      const reply = await invoke<string>("test_llm", { providerId: provider.id });
      testResult.classList.add("ok");
      testResult.textContent = t("llm.reply", reply);
    } catch (e: unknown) {
      testResult.classList.add("err");
      testResult.textContent = `✗ ${errText(e)}`;
    } finally {
      testBtn.disabled = false;
    }
  });
  testRow.append(testBtn, testResult);

  card.append(head, grid, testRow);
  return card;
}

function wireLlmSection(): void {
  $("btn-add-provider").addEventListener("click", () => {
    if (!settings) return;
    const id = `provider_${Date.now().toString(36)}`;
    settings.llm.providers.push({
      id,
      name: t("llm.newProviderName"),
      kind: "openai",
      base_url: "https://api.openai.com/v1",
      api_key: "",
      model: "",
    });
    if (!settings.llm.active_provider_id) settings.llm.active_provider_id = id;
    renderProviderList();
    scheduleSave();
  });
}

// ---------------------------------------------------------------------------
// modes section (CRUD)
// ---------------------------------------------------------------------------

function renderModeList(): void {
  if (!settings) return;
  const host = $("mode-list");
  host.textContent = "";
  if (settings.modes.length === 0) {
    const empty = el("div", "empty-state");
    empty.appendChild(el("span", "empty-icon", "🧩"));
    empty.appendChild(el("span", undefined, t("modes.empty")));
    host.appendChild(empty);
    return;
  }
  for (const mode of settings.modes) {
    host.appendChild(buildModeCard(mode));
  }
}

function buildModeCard(mode: Mode): HTMLElement {
  const card = el("div", "entity-card");
  const isActive = settings?.active_mode_id === mode.id;
  if (isActive) card.classList.add("is-active-entity");

  const head = el("div", "entity-head");

  const nameInput = el("input", "name-input") as HTMLInputElement;
  nameInput.type = "text";
  nameInput.value = mode.name;
  nameInput.placeholder = t("modes.namePlaceholder");
  nameInput.addEventListener("input", () => {
    mode.name = nameInput.value;
    renderHomeModes();
    scheduleSave();
  });

  const useLlmLabel = el("label", "use-llm-row");
  const useLlm = el("input", "switch") as HTMLInputElement;
  useLlm.type = "checkbox";
  useLlm.checked = mode.use_llm;
  useLlmLabel.append(el("span", undefined, t("modes.useLlm")), useLlm);

  const delBtn = el("button", "icon-btn danger", "🗑");
  delBtn.title = t("common.delete");
  delBtn.addEventListener("click", () => {
    if (!settings) return;
    if (settings.modes.length <= 1) {
      toast(t("modes.lastModeError"), true);
      return;
    }
    if (!window.confirm(t("modes.deleteConfirm", mode.name))) return;
    settings.modes = settings.modes.filter((m) => m.id !== mode.id);
    if (settings.active_mode_id === mode.id) {
      settings.active_mode_id = settings.modes[0]?.id ?? "";
    }
    renderModeList();
    renderHomeModes();
    scheduleSave();
  });

  if (isActive) head.appendChild(el("span", "badge accent", t("common.active")));
  head.append(nameInput, useLlmLabel, delBtn);

  const instr = el("textarea") as HTMLTextAreaElement;
  instr.value = mode.instruction;
  instr.placeholder = t("modes.instructionPlaceholder");
  instr.rows = 3;
  instr.disabled = !mode.use_llm;
  instr.addEventListener("input", () => {
    mode.instruction = instr.value;
    scheduleSave();
  });

  useLlm.addEventListener("change", () => {
    mode.use_llm = useLlm.checked;
    instr.disabled = !mode.use_llm;
    renderHomeModes();
    scheduleSave();
  });

  card.append(head, instr);
  return card;
}

function wireModesSection(): void {
  $("btn-add-mode").addEventListener("click", () => {
    if (!settings) return;
    settings.modes.push({
      id: `mode_${Date.now().toString(36)}`,
      name: t("modes.newModeName"),
      instruction: "",
      use_llm: true,
    });
    renderModeList();
    renderHomeModes();
    scheduleSave();
  });
}

// ---------------------------------------------------------------------------
// history section
// ---------------------------------------------------------------------------

function modeNameFor(modeId: string): string {
  return settings?.modes.find((m) => m.id === modeId)?.name ?? modeId;
}

function buildHistoryItem(entry: HistoryEntry, expandable: boolean): HTMLElement {
  const item = el("div", "history-item");

  const head = el("div", "history-head");
  head.appendChild(el("span", "history-time", formatTime(entry.timestamp)));
  head.appendChild(el("span", "history-mode", modeNameFor(entry.mode_id)));
  const actions = el("div", "history-actions");
  const copyBtn = el("button", "icon-btn", "📋");
  copyBtn.title = t("common.copy");
  copyBtn.addEventListener("click", async (e) => {
    e.stopPropagation();
    try {
      await invoke("copy_text", { text: entry.final_text });
      toast(t("history.copied"));
    } catch (err: unknown) {
      toast(t("history.copyFailed", errText(err)), true);
    }
  });
  actions.appendChild(copyBtn);
  head.appendChild(actions);

  const text = el("div", "history-text", entry.final_text);
  item.append(head, text);

  if (expandable) {
    let raw: HTMLElement | null = null;
    item.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest("button")) return;
      const sel = window.getSelection();
      if (sel && sel.toString().length > 0) return;
      if (raw) {
        raw.remove();
        raw = null;
      } else {
        raw = el("div", "history-raw");
        raw.appendChild(el("span", "history-raw-label", t("history.rawLabel")));
        raw.appendChild(document.createTextNode(entry.raw_text));
        item.appendChild(raw);
      }
    });
  }
  return item;
}

function renderHistoryList(): void {
  const host = $("history-list");
  host.textContent = "";
  if (historyEntries.length === 0) {
    const empty = el("div", "empty-state");
    empty.appendChild(el("span", "empty-icon", "🕘"));
    empty.appendChild(el("span", undefined, t("history.empty")));
    host.appendChild(empty);
    return;
  }
  for (const entry of historyEntries) {
    host.appendChild(buildHistoryItem(entry, true));
  }
}

async function refreshHistory(): Promise<void> {
  try {
    historyEntries = await invoke<HistoryEntry[]>("get_history");
  } catch {
    historyEntries = [];
  }
  renderHistoryList();
  renderHomeRecent();
}

function wireHistorySection(): void {
  $("btn-clear-history").addEventListener("click", async () => {
    if (!window.confirm(t("history.clearConfirm"))) return;
    try {
      await invoke("clear_history");
      historyEntries = [];
      renderHistoryList();
      renderHomeRecent();
      toast(t("history.cleared"));
    } catch (e: unknown) {
      toast(t("history.clearFailed", errText(e)), true);
    }
  });
}

// ---------------------------------------------------------------------------
// general section
// ---------------------------------------------------------------------------

function renderGeneralSection(): void {
  if (!settings) return;
  input("in-hotkey").value = displayHotkey(settings.hotkey);
  for (const btn of document.querySelectorAll<HTMLButtonElement>("#hotkey-mode-seg button")) {
    btn.classList.toggle("is-active", btn.dataset.mode === settings.hotkey_mode);
  }
  selectEl("in-ui-lang").value = settings.ui_lang;
  {
    // keep an unknown persisted code selectable instead of showing blank
    const sel = selectEl("in-language");
    const lang = settings.language;
    for (const o of sel.querySelectorAll("option[data-synthetic]")) {
      if ((o as HTMLOptionElement).value !== lang) o.remove();
    }
    if (![...sel.options].some((o) => o.value === lang)) {
      const opt = el("option", undefined, lang) as HTMLOptionElement;
      opt.value = lang;
      opt.dataset.synthetic = "1";
      sel.appendChild(opt);
    }
    sel.value = lang;
  }
  selectEl("in-paste-mode").value = settings.paste_mode;
  input("in-history-limit").value = String(settings.history_limit);
  input("in-restore-clipboard").checked = settings.restore_clipboard;
  input("in-autostart").checked = settings.autostart;
  input("in-live-caption").checked = settings.live_caption;
}

/**
 * Map a keydown to a key name accepted by the global-hotkey 0.8 parser,
 * based on the physical key (e.code) so IME/layout cannot interfere.
 * Returns null for anything else (IME keys, JIS punctuation, dead keys, ...).
 */
function normalizeHotkeyKey(e: KeyboardEvent): string | null {
  const code = e.code;
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1];
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  switch (code) {
    case "Space":
      return "Space";
    case "Enter":
      return "Enter";
    case "Tab":
      return "Tab";
    case "Backspace":
      return "Backspace";
    case "Delete":
      return "Delete";
    case "Insert":
      return "Insert";
    case "Home":
      return "Home";
    case "End":
      return "End";
    case "PageUp":
      return "PageUp";
    case "PageDown":
      return "PageDown";
    // global-hotkey accepts "UP"/"DOWN"/"LEFT"/"RIGHT" aliases for the arrows.
    case "ArrowUp":
      return "Up";
    case "ArrowDown":
      return "Down";
    case "ArrowLeft":
      return "Left";
    case "ArrowRight":
      return "Right";
    default:
      return null;
  }
}

/**
 * Save a new hotkey immediately. The backend validates before persisting and
 * returns Err on a bad/taken hotkey; on rejection, toast the backend message
 * and revert the field and local state to the previous value.
 */
async function applyHotkey(combo: string): Promise<void> {
  if (!settings) return;
  const prev = settings.hotkey;
  settings.hotkey = combo;
  input("in-hotkey").value = displayHotkey(combo);
  $("home-hotkey").textContent = displayHotkey(combo);
  if (combo === prev) return;
  try {
    await persistSettings();
  } catch (e: unknown) {
    toast(errText(e), true);
    if (settings) {
      settings.hotkey = prev;
      input("in-hotkey").value = displayHotkey(prev);
      $("home-hotkey").textContent = displayHotkey(prev);
      // The rejected save also un-queued any coincidental pending edits
      // (persistSettings cleared the debounce timer); re-arm so they retry
      // without the bad hotkey.
      scheduleSave();
    }
  }
}

/** e.code → special single-modifier hotkey token (SPEC v0.3). */
const MOD_CODE_TOKEN: Record<string, string> = {
  AltRight: "RAlt",
  AltLeft: "LAlt",
  ControlRight: "RCtrl",
  ControlLeft: "LCtrl",
  ShiftRight: "RShift",
  ShiftLeft: "LShift",
};

function wireGeneralSection(): void {
  const hotkeyInput = input("in-hotkey");
  /** e.code of a lone modifier currently held down in the capture field; its
   * keyup (with no other key in between) confirms a bare-modifier hotkey. */
  let pendingModCode: string | null = null;
  /** e.codes of all modifiers currently held down in the capture field
   * (SPEC v0.3.1 F1). A bare-modifier tap confirms only if, from its keydown
   * to its keyup, this set never contained more than that single code — so
   * holding both Shifts is never misdetected as a tap. */
  const heldModCodes = new Set<string>();

  hotkeyInput.addEventListener("focus", () => {
    hotkeyInput.classList.add("capturing");
    hotkeyInput.value = t("hotkey.press");
    pendingModCode = null;
    heldModCodes.clear();
    // Suspend the global hotkey path while capturing so pressing the current
    // hotkey doesn't start a recording (fire-and-forget; failures ignored).
    void invoke("set_hotkey_suspended", { suspended: true }).catch(() => {});
  });
  hotkeyInput.addEventListener("blur", () => {
    hotkeyInput.classList.remove("capturing");
    pendingModCode = null;
    heldModCodes.clear();
    if (settings) hotkeyInput.value = displayHotkey(settings.hotkey);
    void invoke("set_hotkey_suspended", { suspended: false }).catch(() => {});
  });
  hotkeyInput.addEventListener("keydown", (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (!settings) return;
    if (e.key === "Escape") {
      pendingModCode = null;
      heldModCodes.clear();
      hotkeyInput.blur();
      return;
    }
    const mods: string[] = [];
    if (e.ctrlKey) mods.push("Ctrl");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    // AltGraph (e.code AltRight) is a modifier too (SPEC v0.3.1 F2): it maps
    // to RAlt for bare-tap detection and must not hit the "invalid key"
    // rejection. On AltGr layouts the backend rejects RAlt
    // (ERR_HOTKEY_REGISTER|RAlt) and the applyHotkey rollback handles it.
    if (
      e.key === "Control" ||
      e.key === "Alt" ||
      e.key === "Shift" ||
      e.key === "Meta" ||
      e.key === "AltGraph"
    ) {
      heldModCodes.add(e.code);
      const token = MOD_CODE_TOKEN[e.code];
      if (token !== undefined && heldModCodes.size === 1) {
        // A lone modifier went down: pending bare-modifier tap. If its keyup
        // arrives while it stayed the only held modifier, it becomes the
        // hotkey.
        pendingModCode = e.code;
        hotkeyInput.value = `${displayHotkey(token)}…`;
      } else {
        // A second modifier joined (or an unmapped one, e.g. Meta): this is a
        // combo in progress, not a bare tap.
        pendingModCode = null;
        hotkeyInput.value = mods.length > 0 ? `${mods.join("+")}+…` : t("hotkey.press");
      }
      return;
    }
    // Any non-modifier key cancels a pending bare-modifier tap: combo path.
    pendingModCode = null;
    const key = normalizeHotkeyKey(e);
    if (key === null) {
      hotkeyInput.value = t("hotkey.invalid");
      return;
    }
    const isFKey = /^F([1-9]|1[0-9]|2[0-4])$/.test(key);
    if (mods.length === 0 && !isFKey) {
      hotkeyInput.value = t("hotkey.needModifier");
      return;
    }
    const combo = [...mods, key].join("+");
    void applyHotkey(combo);
    hotkeyInput.blur();
  });
  hotkeyInput.addEventListener("keyup", (e) => {
    e.preventDefault();
    e.stopPropagation();
    heldModCodes.delete(e.code);
    if (!settings) return;
    const token = MOD_CODE_TOKEN[e.code];
    if (pendingModCode !== null && e.code === pendingModCode && token !== undefined) {
      // Bare-modifier tap confirmed: from its keydown to this keyup the held
      // set never contained more than this single code (any other keydown
      // cleared pendingModCode). applyHotkey validates via the backend and
      // rolls back on Err.
      pendingModCode = null;
      void applyHotkey(token);
      hotkeyInput.blur();
    }
  });

  $("hotkey-mode-seg").addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-mode]");
    if (!btn || !settings) return;
    settings.hotkey_mode = btn.dataset.mode as Settings["hotkey_mode"];
    renderGeneralSection();
    scheduleSave();
  });

  const uiLangSel = selectEl("in-ui-lang");
  const autoOpt = el("option", undefined, t("general.langAuto")) as HTMLOptionElement;
  autoOpt.value = "";
  autoOpt.dataset.i18n = "general.langAuto";
  uiLangSel.appendChild(autoOpt);
  for (const lang of LANGS) {
    const opt = el("option", undefined, lang.native) as HTMLOptionElement;
    opt.value = lang.code;
    uiLangSel.appendChild(opt);
  }
  uiLangSel.addEventListener("change", () => {
    if (!settings) return;
    settings.ui_lang = uiLangSel.value; // "" = auto (follow OS locale)
    scheduleSave();
    // Re-render immediately (no restart). Pending edits are safe: every edit
    // mutates `settings` synchronously, and renderAll() rebuilds inputs from
    // that same live object — only the (debounced) save is deferred.
    void applyLanguageSetting(uiLangSel.value);
  });

  // STT language: whisper language codes (SPEC v0.5). "auto" = whisper
  // detects the language itself, which is error-prone on short utterances.
  const sttLangSel = selectEl("in-language");
  const detectOpt = el("option", undefined, t("general.langDetect")) as HTMLOptionElement;
  detectOpt.value = "auto";
  detectOpt.dataset.i18n = "general.langDetect";
  sttLangSel.appendChild(detectOpt);
  for (const lang of STT_LANGS) {
    const opt = el("option", undefined, lang.label) as HTMLOptionElement;
    opt.value = lang.code;
    sttLangSel.appendChild(opt);
  }
  selectEl("in-language").addEventListener("change", () => {
    if (!settings) return;
    settings.language = selectEl("in-language").value;
    scheduleSave();
  });
  selectEl("in-paste-mode").addEventListener("change", () => {
    if (!settings) return;
    settings.paste_mode = selectEl("in-paste-mode").value as Settings["paste_mode"];
    scheduleSave();
  });
  input("in-history-limit").addEventListener("input", () => {
    if (!settings) return;
    const v = parseInt(input("in-history-limit").value, 10);
    if (Number.isFinite(v) && v >= 0) {
      settings.history_limit = v;
      scheduleSave();
    }
  });
  input("in-restore-clipboard").addEventListener("change", () => {
    if (!settings) return;
    settings.restore_clipboard = input("in-restore-clipboard").checked;
    scheduleSave();
  });
  input("in-live-caption").addEventListener("change", () => {
    if (!settings) return;
    settings.live_caption = input("in-live-caption").checked;
    scheduleSave();
  });
  input("in-autostart").addEventListener("change", () => {
    if (!settings) return;
    settings.autostart = input("in-autostart").checked;
    scheduleSave();
  });
}

// ---------------------------------------------------------------------------
// setup section
// ---------------------------------------------------------------------------

function downloadKey(kind: string, name: string): string {
  return `${kind}:${name}`;
}

const ACCEL_LABEL_KEYS: Record<string, string> = {
  auto: "stt.accelAuto",
  cuda: "stt.accelCuda",
  vulkan: "stt.accelVulkan",
  npu: "stt.accelNpu",
  cpu: "stt.accelCpu",
};

/** Short, untranslated build names for buttons and the resolved-device line. */
const ACCEL_SHORT: Record<string, string> = { cuda: "CUDA", vulkan: "Vulkan", npu: "NPU", cpu: "CPU" };

const BACKEND_BADGE_KEYS: Record<string, string> = {
  cuda: "setup.backendCuda",
  vulkan: "setup.backendVulkan",
  npu: "setup.backendNpu",
  cpu: "setup.backendCpu",
};

/** Mirror of `hw::build_satisfies`: CPU runs on the CUDA/Vulkan builds too. */
function buildSatisfies(installed: string, wanted: string): boolean {
  return installed === wanted || (wanted === "cpu" && (installed === "cuda" || installed === "vulkan"));
}

/** Accelerator selector + detected-hardware line inside the server card (v0.9). */
function renderAccel(s: SetupStatus): void {
  const select = $("in-local-accel") as HTMLSelectElement;
  const current = settings?.stt.local.accel || "auto";
  const options = s.accel_options.length ? s.accel_options : ["auto", "cpu"];
  select.replaceChildren(
    ...options.map((v) => {
      const opt = document.createElement("option");
      opt.value = v;
      opt.textContent = t(ACCEL_LABEL_KEYS[v] ?? "stt.accelAuto");
      return opt;
    }),
  );
  select.value = options.includes(current) ? current : "auto";

  const found: string[] = s.hw.gpus.length ? [...s.hw.gpus] : [t("stt.noGpu")];
  if (s.hw.npu) found.push(t("stt.npuFound"));
  const resolved = ACCEL_SHORT[s.effective_accel] ?? "CPU";
  $("accel-hint").textContent = `${t("stt.accelDetected", found.join(" / "))} → ${resolved}`;
}

/** whisper-server card at the top of the local STT panel (was the Setup section). */
function renderServerCard(): void {
  const s = setupStatus;
  const serverBadge = $("server-badge");
  const backendBadge = $("server-backend-badge");
  const buildHint = $("server-build-hint");
  const serverPath = $("server-path");
  const installBtn = $("btn-install-server") as HTMLButtonElement;
  const npuRow = $("npu-cache-row");

  if (!s) {
    serverBadge.textContent = t("setup.checking");
    serverBadge.className = "badge";
    backendBadge.hidden = true;
    buildHint.hidden = true;
    npuRow.hidden = true;
    serverPath.textContent = "—";
    installBtn.disabled = true;
    return;
  }

  serverBadge.textContent = s.server_installed ? t("common.installed") : t("common.notInstalled");
  serverBadge.className = `badge${s.server_installed ? " ok" : ""}`;
  serverPath.textContent = s.server_path || "—";
  renderAccel(s);

  // Which build is installed, and whether the selected accelerator needs a
  // different one (the backend also fetches it on its own after a change).
  const wanted = s.effective_accel;
  const backend = s.server_backend || "cpu";
  backendBadge.hidden = !s.server_installed;
  backendBadge.textContent = t(BACKEND_BADGE_KEYS[backend] ?? "setup.backendCpu");
  backendBadge.className = `badge${backend !== "cpu" ? " accent" : ""}`;
  const offerBuild = s.server_installed && !buildSatisfies(backend, wanted);
  const wantedLabel = ACCEL_SHORT[wanted] ?? "CPU";
  buildHint.hidden = !offerBuild;
  buildHint.textContent = offerBuild ? t("setup.buildHint", wantedLabel) : "";
  installBtn.textContent = offerBuild
    ? t("setup.installBuild", wantedLabel)
    : s.server_installed
      ? t("setup.reinstall")
      : t("setup.install");
  installBtn.className = `btn btn-sm${offerBuild || !s.server_installed ? " btn-primary" : ""}`;
  installBtn.disabled = activeDownloads.has(downloadKey("server", "whisper-server"));

  npuRow.hidden = !(wanted === "npu" && s.model_installed && !s.npu_cache_ready);
  ($("btn-install-npu") as HTMLButtonElement).disabled = activeDownloads.has(
    downloadKey("npu", "npu-encoder"),
  );
}

async function startNpuCacheDownload(): Promise<void> {
  const key = downloadKey("npu", "npu-encoder");
  activeDownloads.set(key, { kind: "npu", name: "npu-encoder", downloaded: 0, total: 0, done: false });
  const wrap = $("npu-progress");
  wrap.hidden = false;
  wrap.classList.add("indeterminate");
  ($("btn-install-npu") as HTMLButtonElement).disabled = true;
  try {
    await invoke("download_npu_cache");
  } catch (e: unknown) {
    // If the key is gone, a download-progress done/error event already
    // handled (and toasted) this failure — don't toast twice.
    const stillPending = activeDownloads.has(key);
    activeDownloads.delete(key);
    wrap.hidden = true;
    ($("btn-install-npu") as HTMLButtonElement).disabled = false;
    if (stillPending) toast(t("setup.npuCacheFailed", errText(e)), true);
  }
}

function buildProgressEl(key: string): HTMLElement {
  const wrap = el("div", "progress");
  wrap.dataset.downloadKey = key;
  wrap.hidden = !activeDownloads.has(key);
  wrap.appendChild(el("div", "progress-bar"));
  wrap.appendChild(el("span", "progress-text"));
  const current = activeDownloads.get(key);
  if (current) applyProgress(wrap, current);
  return wrap;
}

function applyProgress(wrap: HTMLElement, p: DownloadProgressPayload): void {
  const bar = wrap.querySelector<HTMLElement>(".progress-bar");
  const text = wrap.querySelector<HTMLElement>(".progress-text");
  if (!bar || !text) return;
  wrap.hidden = false;
  if (p.total > 0) {
    wrap.classList.remove("indeterminate");
    const pct = Math.min(100, (p.downloaded / p.total) * 100);
    bar.style.width = `${pct.toFixed(1)}%`;
    text.textContent = `${formatBytes(p.downloaded)} / ${formatBytes(p.total)} (${pct.toFixed(0)}%)`;
  } else {
    wrap.classList.add("indeterminate");
    text.textContent = formatBytes(p.downloaded);
  }
}

async function startServerDownload(): Promise<void> {
  const key = downloadKey("server", "whisper-server");
  activeDownloads.set(key, {
    kind: "server",
    name: "whisper-server",
    downloaded: 0,
    total: 0,
    done: false,
  });
  const wrap = $("server-progress");
  wrap.dataset.downloadKey = key;
  wrap.hidden = false;
  wrap.classList.add("indeterminate");
  ($("btn-install-server") as HTMLButtonElement).disabled = true;
  try {
    await invoke("download_whisper_server");
  } catch (e: unknown) {
    // If the key is gone, a download-progress done/error event already
    // handled (and toasted) this failure — don't toast twice.
    const stillPending = activeDownloads.has(key);
    activeDownloads.delete(key);
    wrap.hidden = true;
    ($("btn-install-server") as HTMLButtonElement).disabled = false;
    if (stillPending) toast(t("setup.serverInstallFailed", errText(e)), true);
  }
}

async function startModelDownload(name: string): Promise<void> {
  const key = downloadKey("model", name);
  activeDownloads.set(key, {
    kind: "model",
    name,
    downloaded: 0,
    total: 0,
    done: false,
  });
  renderSttModelList();
  try {
    await invoke("download_model", { model: name });
  } catch (e: unknown) {
    // If the key is gone, a download-progress done/error event already
    // handled (and toasted) this failure — don't toast twice.
    const stillPending = activeDownloads.has(key);
    activeDownloads.delete(key);
    renderSttModelList();
    if (stillPending) toast(t("setup.modelDownloadFailed", name, errText(e)), true);
  }
}

function onDownloadProgress(p: DownloadProgressPayload): void {
  const key = downloadKey(p.kind, p.kind === "server" ? "whisper-server" : p.name);
  if (p.done) {
    activeDownloads.delete(key);
    if (p.error) {
      toast(
        p.kind === "server"
          ? t("setup.serverInstallFailed", tMsg(p.error))
          : p.kind === "npu"
            ? t("setup.npuCacheFailed", tMsg(p.error))
            : t("setup.modelDownloadFailed", p.name, tMsg(p.error)),
        true,
      );
    } else {
      toast(
        p.kind === "server"
          ? t("setup.serverInstalled")
          : p.kind === "npu"
            ? t("setup.npuCacheDownloaded")
            : t("setup.modelDownloaded", p.name),
      );
    }
    if (p.kind === "server" || p.kind === "npu") {
      $(p.kind === "server" ? "server-progress" : "npu-progress").hidden = true;
    }
    void refreshSetupStatus();
    return;
  }
  activeDownloads.set(key, p);
  // the home setup card shows the same downloads as its steps
  const onboardBar = document.querySelector<HTMLElement>(
    `#onboard-steps .progress[data-download-key="${key}"]`,
  );
  if (onboardBar) applyProgress(onboardBar, p);
  if (p.kind === "server" || p.kind === "npu") {
    // the server build and the NPU encoder can download at the same time,
    // each with its own bar
    applyProgress($(p.kind === "server" ? "server-progress" : "npu-progress"), p);
    const btn = p.kind === "server" ? "btn-install-server" : "btn-install-npu";
    ($(btn) as HTMLButtonElement).disabled = true;
  } else {
    const wrap = document.querySelector<HTMLElement>(
      `#stt-model-list .progress[data-download-key="${key}"]`,
    );
    if (wrap) {
      applyProgress(wrap, p);
    } else {
      renderSttModelList();
    }
  }
}

async function refreshSetupStatus(): Promise<void> {
  try {
    setupStatus = await invoke<SetupStatus>("setup_status");
  } catch {
    setupStatus = null;
  }
  renderServerCard();
  renderSttModelList();
  renderOnboard();
  renderHomeStatus();
}

function wireServerCard(): void {
  $("btn-install-server").addEventListener("click", () => void startServerDownload());
  $("btn-install-npu").addEventListener("click", () => void startNpuCacheDownload());
  // Save right away (not debounced) so setup_status reflects the choice and
  // the backend starts fetching a matching build if one is needed (v0.9).
  $("in-local-accel").addEventListener("change", async () => {
    if (!settings) return;
    const v = ($("in-local-accel") as HTMLSelectElement).value;
    settings.stt.local.accel = v;
    settings.stt.local.gpu = v !== "cpu";
    try {
      await persistSettings();
    } catch (e: unknown) {
      toast(errText(e), true);
    }
    void refreshSetupStatus();
  });
  $("btn-open-config").addEventListener("click", async () => {
    try {
      await invoke("open_config_dir");
    } catch (e: unknown) {
      toast(errText(e), true);
    }
  });
}

// ---------------------------------------------------------------------------
// update section
// ---------------------------------------------------------------------------

async function openUpdateUrl(url: string): Promise<void> {
  try {
    await invoke("open_url", { url });
  } catch (e: unknown) {
    toast(errText(e), true);
  }
}

function renderUpdateSection(): void {
  if (!settings) return;
  input("in-auto-check").checked = settings.update.auto_check;
  input("in-update-owner").value = settings.update.owner;
  input("in-update-repo").value = settings.update.repo;
  renderUpdateResult();
}

/** True while install_update runs; both update buttons stay disabled. */
let updateInstalling = false;
/** Set after a failed install so the download-page fallback becomes visible. */
let updateInstallFailed = false;

function formatProgress(p: UpdateProgressPayload): string {
  if (p.phase === "install") return t("update.installing");
  const done = p.downloaded ?? 0;
  if (p.total && p.total > 0) {
    return t("update.downloading", `${Math.min(100, Math.round((done / p.total) * 100))}%`);
  }
  return t("update.downloading", `${(done / (1024 * 1024)).toFixed(1)} MB`);
}

function setUpdateButtonsDisabled(disabled: boolean): void {
  for (const id of [
    "btn-banner-install",
    "btn-banner-open",
    "btn-install-update",
    "btn-open-download",
    "btn-check-update",
  ]) {
    ($(id) as HTMLButtonElement).disabled = disabled;
  }
}

/**
 * One-click update (SPEC v0.6): the backend downloads, verifies and installs
 * the latest release and relaunches; the UI only mirrors `update-progress`.
 * On failure the download-page buttons appear as a manual fallback.
 */
async function startInstallUpdate(): Promise<void> {
  if (updateInstalling) return;
  updateInstalling = true;
  updateInstallFailed = false;
  setUpdateButtonsDisabled(true);
  // a retry hides the fallback from the previous failure
  ($("btn-banner-open") as HTMLButtonElement).hidden = true;
  ($("btn-open-download") as HTMLButtonElement).hidden = true;
  const initial = t("update.downloading", "0%");
  $("update-install-result").className = "test-result";
  $("update-install-result").textContent = initial;
  $("update-banner-text").textContent = initial;
  try {
    await invoke("install_update");
    // On success the process is replaced (Windows: installer, Linux: restart).
  } catch (e: unknown) {
    updateInstalling = false;
    updateInstallFailed = true;
    setUpdateButtonsDisabled(false);
    const msg = errText(e);
    $("update-install-result").classList.add("err");
    $("update-install-result").textContent = `✗ ${msg}`;
    toast(msg, true);
    renderUpdateResult();
    if (bannerInfo) {
      $("update-banner-text").textContent = t("update.available", bannerInfo.latest);
      ($("btn-banner-open") as HTMLButtonElement).hidden = false;
    }
  }
}

function renderUpdateResult(): void {
  const panel = $("update-result");
  if (!updateInfo) {
    panel.hidden = true;
    return;
  }
  panel.hidden = false;
  const title = $("update-result-title");
  if (updateInfo.update_available) {
    title.textContent = t("update.available", updateInfo.latest);
    title.className = "update-result-title is-new";
  } else {
    title.textContent = t("update.upToDate", updateInfo.latest);
    title.className = "update-result-title";
  }
  // Release notes are untrusted remote content — textContent only, never HTML.
  $("update-notes").textContent = updateInfo.notes;
  $("update-notes-wrap").hidden = updateInfo.notes.trim() === "";
  ($("btn-install-update") as HTMLButtonElement).hidden = !updateInfo.update_available;
  ($("btn-open-download") as HTMLButtonElement).hidden =
    !(updateInfo.update_available && updateInstallFailed);
}

function showUpdateBanner(info: UpdateInfo): void {
  bannerInfo = info;
  $("update-banner-text").textContent = t("update.available", info.latest);
  $("update-banner").hidden = false;
}

function wireUpdateSection(): void {
  const checkBtn = $("btn-check-update") as HTMLButtonElement;
  const checkResult = $("update-check-result");
  checkBtn.addEventListener("click", async () => {
    checkBtn.disabled = true;
    checkResult.className = "test-result";
    checkResult.textContent = t("update.checking");
    try {
      await doSave();
      updateInfo = await invoke<UpdateInfo>("check_update");
      checkResult.textContent = "";
      renderUpdateResult();
    } catch (e: unknown) {
      checkResult.classList.add("err");
      checkResult.textContent = `✗ ${errText(e)}`;
    } finally {
      checkBtn.disabled = false;
    }
  });

  $("btn-open-download").addEventListener("click", () => {
    if (updateInfo) void openUpdateUrl(updateInfo.url);
  });
  $("btn-install-update").addEventListener("click", () => {
    void startInstallUpdate();
  });

  $("btn-banner-install").addEventListener("click", () => {
    void startInstallUpdate();
  });
  $("btn-banner-open").addEventListener("click", () => {
    if (bannerInfo) void openUpdateUrl(bannerInfo.url);
  });
  void listen<UpdateProgressPayload>("update-progress", (event) => {
    if (!updateInstalling) return;
    const text = formatProgress(event.payload);
    $("update-install-result").textContent = text;
    $("update-banner-text").textContent = text;
  });
  $("btn-banner-close").addEventListener("click", () => {
    $("update-banner").hidden = true;
    bannerInfo = null;
  });

  input("in-auto-check").addEventListener("change", () => {
    if (!settings) return;
    settings.update.auto_check = input("in-auto-check").checked;
    scheduleSave();
  });
  input("in-update-owner").addEventListener("input", () => {
    if (!settings) return;
    settings.update.owner = input("in-update-owner").value.trim();
    scheduleSave();
  });
  input("in-update-repo").addEventListener("input", () => {
    if (!settings) return;
    settings.update.repo = input("in-update-repo").value.trim();
    scheduleSave();
  });
}

/** Silent auto-check at startup (SPEC): failures are ignored; a newer version
 * shows a dismissible banner on the home section. */
async function autoCheckUpdate(): Promise<void> {
  try {
    const info = await invoke<UpdateInfo>("check_update");
    updateInfo = info;
    renderUpdateResult();
    if (info.update_available) showUpdateBanner(info);
  } catch {
    // silent by contract
  }
}

// ---------------------------------------------------------------------------
// full re-render
// ---------------------------------------------------------------------------

function renderAll(): void {
  if (!settings) return;
  renderHome();
  renderSttSection();
  renderProviderList();
  renderModeList();
  renderGeneralSection();
  renderServerCard();
  renderUpdateSection();
}

/**
 * Switch the UI language in place: re-translate static DOM, then re-render all
 * dynamically built strings. Input values are re-read from the live `settings`
 * object (which already holds any pending edits), so nothing is lost.
 */
function applyLanguage(code: string): void {
  setLang(code);
  applyDom();
  setStatusPill(lastStatusPayload);
  renderAll();
  renderHistoryList();
  // Persistent one-shot result lines cannot be re-translated (they hold
  // free-form past results) — clear them instead of showing stale language.
  for (const id of ["stt-test-result", "update-check-result"]) {
    const node = document.getElementById(id);
    if (node) node.textContent = "";
  }
  if (bannerInfo) {
    $("update-banner-text").textContent = t("update.available", bannerInfo.latest);
  }
}

/**
 * Resolve a ui_lang SETTING value ("" = auto → ask the backend for the OS
 * locale) and apply it. Falls back to the current language on failure.
 */
async function applyLanguageSetting(value: string): Promise<void> {
  let code = value;
  if (!code) {
    try {
      code = await invoke<string>("get_ui_lang");
    } catch {
      code = getLang();
    }
  }
  if (code !== getLang()) {
    applyLanguage(code);
  } else {
    renderAll();
  }
}

// ---------------------------------------------------------------------------
// events + init
// ---------------------------------------------------------------------------

async function setupListeners(): Promise<void> {
  await listen<StatusChangedPayload>("status-changed", (event) => {
    setStatusPill(event.payload);
  });

  await listen<Settings>("settings-changed", (event) => {
    // Adopt active_mode_id unconditionally first — the tray changes it
    // autonomously, and the echo suppression below must never swallow it.
    if (settings && event.payload.active_mode_id !== settings.active_mode_id) {
      settings.active_mode_id = event.payload.active_mode_id;
      renderHomeModes();
      renderModeList();
    }
    // Ignore echoes of our own recent saves, and no-op updates (compared with
    // active_mode_id stripped). Keep the current settings object so live input
    // closures (provider/mode cards) stay valid.
    const incoming = snapshotKey(event.payload);
    if (savedSnapshots.includes(incoming) || (settings && incoming === snapshotKey(settings))) {
      return;
    }
    // Pending local edits (debounced save armed, or a dirty API key not yet
    // blurred): don't wholesale-replace; active_mode_id was already merged.
    const hasPendingEdits = saveTimer !== undefined || apiKeyDirty;
    if (hasPendingEdits && settings) {
      return;
    }
    settings = event.payload;
    recordSavedSnapshot(incoming);
    // "" = auto: resolve via backend, then re-render either way.
    void applyLanguageSetting(settings.ui_lang);
  });

  await listen<null>("history-updated", () => {
    void refreshHistory();
  });

  // whisper-server started / stopped: the home status card shows it
  await listen<boolean>("server-state", () => {
    void refreshSetupStatus();
  });

  await listen<DownloadProgressPayload>("download-progress", (event) => {
    onDownloadProgress(event.payload);
  });
}

async function init(): Promise<void> {
  setupNav();
  wireSttSection();
  wireLlmSection();
  wireModesSection();
  wireHistorySection();
  wireGeneralSection();
  wireServerCard();
  wireOnboard();
  wireUpdateSection();

  $("btn-record-test").addEventListener("click", async () => {
    try {
      await invoke("toggle_recording");
    } catch (e: unknown) {
      toast(errText(e), true);
    }
  });

  $("btn-quit").addEventListener("click", async () => {
    if (!window.confirm(t("quit.confirm"))) return;
    try {
      await invoke("quit_app");
    } catch (e: unknown) {
      toast(errText(e), true);
    }
  });

  await setupListeners();

  try {
    settings = await invoke<Settings>("get_settings");
    recordSavedSnapshot(snapshotKey(settings));
    // ui_lang "" means auto — ask the backend for the OS-resolved code.
    let lang = settings.ui_lang;
    if (!lang) {
      try {
        lang = await invoke<string>("get_ui_lang");
      } catch {
        lang = "en";
      }
    }
    setLang(lang);
  } catch (e: unknown) {
    toast(t("err.loadSettings", errText(e)), true);
  }
  applyDom();

  try {
    const version = await getVersion();
    $("app-version").textContent = `v${version}`;
    $("update-current-version").textContent = `v${version}`;
  } catch {
    // non-fatal: version chips stay empty
  }

  try {
    const status = await invoke<string>("get_status");
    setStatusPill({ status: status as AppStatus });
  } catch {
    setStatusPill({ status: "idle" });
  }

  // Errors that happened before this page could listen (e.g. the saved hotkey
  // is taken by another app at startup).
  try {
    const startupError = await invoke<string | null>("get_startup_error");
    if (startupError) toast(tMsg(startupError), true);
  } catch {
    // non-fatal
  }

  renderAll();
  void refreshHistory();
  void refreshSetupStatus();

  if (settings?.update.auto_check) {
    void autoCheckUpdate();
  }
}

window.addEventListener("DOMContentLoaded", () => {
  void init();
});
