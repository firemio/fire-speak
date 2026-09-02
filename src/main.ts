// fire-speak — settings window (index.html)

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
} from "./types";

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
  return d.toLocaleString("ja-JP", {
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
let lastSavedJson = "";
let apiKeyDirty = false;

const activeDownloads = new Map<string, DownloadProgressPayload>();

const STATUS_LABELS: Record<AppStatus, string> = {
  idle: "待機中",
  recording: "録音中",
  transcribing: "文字起こし中…",
  polishing: "AI整形中…",
  done: "完了",
  error: "エラー",
};

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

async function doSave(): Promise<void> {
  if (!settings) return;
  if (saveTimer !== undefined) {
    window.clearTimeout(saveTimer);
    saveTimer = undefined;
  }
  apiKeyDirty = false;
  lastSavedJson = JSON.stringify(settings);
  try {
    await invoke("save_settings", { settings });
  } catch (e: unknown) {
    toast(`保存に失敗しました: ${errMsg(e)}`, true);
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

const SECTION_TITLES: Record<string, string> = {
  home: "ホーム",
  stt: "音声認識",
  llm: "AI整形",
  modes: "モード",
  history: "履歴",
  general: "一般",
  setup: "セットアップ",
};

function showSection(name: string): void {
  for (const btn of document.querySelectorAll<HTMLButtonElement>(".nav-item")) {
    btn.classList.toggle("is-active", btn.dataset.section === name);
  }
  for (const sec of document.querySelectorAll<HTMLElement>(".section")) {
    sec.classList.toggle("is-active", sec.id === `section-${name}`);
  }
  $("section-title").textContent = SECTION_TITLES[name] ?? name;
  if (name === "home") {
    renderHome();
  } else if (name === "history") {
    void refreshHistory();
  } else if (name === "setup" || name === "stt") {
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
  const pill = $("status-pill");
  pill.dataset.status = payload.status;
  const label =
    payload.status === "error" && payload.message
      ? `エラー: ${payload.message}`
      : payload.message && payload.status !== "done"
        ? `${STATUS_LABELS[payload.status]} — ${payload.message}`
        : STATUS_LABELS[payload.status];
  $("status-label").textContent = label;
  pill.title = payload.message ?? "";
}

// ---------------------------------------------------------------------------
// home
// ---------------------------------------------------------------------------

function renderHome(): void {
  if (!settings) return;
  $("home-hotkey").textContent = settings.hotkey;
  renderHomeModes();
  renderHomeRecent();
}

function renderHomeModes(): void {
  if (!settings) return;
  const host = $("home-modes");
  host.textContent = "";
  for (const mode of settings.modes) {
    const card = el("button", "mode-card");
    card.type = "button";
    if (mode.id === settings.active_mode_id) card.classList.add("is-active");
    const name = el("div", "mode-card-name", mode.name);
    const sub = el(
      "div",
      "mode-card-sub",
      mode.use_llm ? mode.instruction || "AI整形" : "AI整形なし (そのまま入力)",
    );
    card.append(name, sub);
    if (mode.id === settings.active_mode_id) {
      card.appendChild(el("span", "mode-card-check", "✓"));
    }
    card.addEventListener("click", () => void activateMode(mode.id));
    host.appendChild(card);
  }
}

async function activateMode(modeId: string): Promise<void> {
  if (!settings) return;
  settings.active_mode_id = modeId;
  renderHomeModes();
  try {
    await invoke("set_active_mode", { modeId });
  } catch (e: unknown) {
    toast(`モードの切替に失敗しました: ${errMsg(e)}`, true);
  }
}

function renderHomeRecent(): void {
  const host = $("home-recent");
  host.textContent = "";
  const recent = historyEntries.slice(0, 3);
  if (recent.length === 0) {
    const empty = el("div", "empty-state");
    empty.appendChild(el("span", "empty-icon", "🎤"));
    empty.appendChild(
      el("span", undefined, "まだ履歴がありません。ホットキーで音声入力を試してみましょう。"),
    );
    host.appendChild(empty);
    return;
  }
  for (const entry of recent) {
    host.appendChild(buildHistoryItem(entry, false));
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

/** Derive the managed models directory from setup_status.model_path if possible. */
function managedModelPathFor(name: string): string | null {
  if (!setupStatus || !setupStatus.model_path) return null;
  const p = setupStatus.model_path;
  const idx = Math.max(p.lastIndexOf("\\"), p.lastIndexOf("/"));
  if (idx < 0) return null;
  const sep = p.includes("\\") ? "\\" : "/";
  return `${p.slice(0, idx)}${sep}ggml-${name}.bin`;
}

function renderSttModelList(): void {
  if (!settings) return;
  const host = $("stt-model-list");
  host.textContent = "";
  const models = setupStatus?.models ?? [];
  if (models.length === 0) {
    host.appendChild(el("div", "hint", "モデル情報を取得中…"));
    return;
  }
  const currentPath = settings.stt.local.model_path;

  const addOption = (
    label: string,
    meta: string,
    selected: boolean,
    disabled: boolean,
    onSelect: (() => void) | null,
  ): void => {
    const opt = el("div", "model-option");
    if (selected) opt.classList.add("is-selected");
    if (disabled) opt.classList.add("is-disabled");
    opt.appendChild(el("span", "radio"));
    opt.appendChild(el("span", "model-option-name", label));
    opt.appendChild(el("span", "model-option-meta", meta));
    if (!disabled && onSelect) {
      opt.addEventListener("click", onSelect);
    }
    host.appendChild(opt);
  };

  addOption(
    "自動 (インストール済みの既定モデル)",
    setupStatus?.model_installed ? "利用可能" : "モデル未インストール",
    currentPath === "",
    false,
    () => {
      if (!settings) return;
      settings.stt.local.model_path = "";
      input("in-local-model-path").value = "";
      renderSttModelList();
      scheduleSave();
    },
  );

  for (const m of models) {
    const managed = managedModelPathFor(m.name);
    const selected = managed !== null && currentPath === managed && currentPath !== "";
    addOption(
      m.name,
      m.installed ? `${m.size_mb} MB ・ インストール済み` : `${m.size_mb} MB ・ 未インストール`,
      selected,
      !m.installed || managed === null,
      () => {
        if (!settings || managed === null) return;
        settings.stt.local.model_path = managed;
        input("in-local-model-path").value = managed;
        renderSttModelList();
        scheduleSave();
      },
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

  $("link-goto-setup").addEventListener("click", (e) => {
    e.preventDefault();
    showSection("setup");
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
    result.textContent = "テスト中…";
    try {
      await doSave();
      const msg = await invoke<string>("test_stt");
      result.classList.add("ok");
      result.textContent = `✓ ${msg}`;
    } catch (e: unknown) {
      result.classList.add("err");
      result.textContent = `✗ ${errMsg(e)}`;
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
    empty.appendChild(el("span", undefined, "プロバイダがありません。「＋ プロバイダを追加」から作成してください。"));
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
  radioLabel.append(radio, el("span", undefined, "アクティブ"));

  const nameInput = el("input", "name-input") as HTMLInputElement;
  nameInput.type = "text";
  nameInput.value = provider.name;
  nameInput.placeholder = "プロバイダ名";
  nameInput.addEventListener("input", () => {
    provider.name = nameInput.value;
    scheduleSave();
  });

  const delBtn = el("button", "icon-btn danger", "🗑");
  delBtn.title = "削除";
  delBtn.addEventListener("click", () => {
    if (!settings) return;
    if (!window.confirm(`プロバイダ「${provider.name}」を削除しますか?`)) return;
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
  kindField.appendChild(el("span", "field-label", "種別"));
  const kindSel = el("select") as HTMLSelectElement;
  for (const [value, label] of [
    ["anthropic", "Anthropic (Claude)"],
    ["openai", "OpenAI互換 (chat/completions)"],
  ] as const) {
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
  modelField.appendChild(el("span", "field-label", "モデル"));
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
  urlField.appendChild(el("span", "field-label", "Base URL"));
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
  keyField.appendChild(el("span", "field-label", "APIキー"));
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
  eyeBtn.title = "表示切替";
  eyeBtn.addEventListener("click", () => {
    keyInput.type = keyInput.type === "password" ? "text" : "password";
  });
  keyWrap.append(keyInput, eyeBtn);
  keyField.appendChild(keyWrap);

  grid.append(kindField, modelField, urlField, keyField);

  // test row
  const testRow = el("div", "test-row inline-test");
  const testBtn = el("button", "btn btn-sm", "🔌 テスト") as HTMLButtonElement;
  const testResult = el("span", "test-result");
  testBtn.addEventListener("click", async () => {
    testBtn.disabled = true;
    testResult.className = "test-result";
    testResult.textContent = "テスト中…";
    try {
      await doSave();
      const reply = await invoke<string>("test_llm", { providerId: provider.id });
      testResult.classList.add("ok");
      testResult.textContent = `✓ 応答: ${reply}`;
    } catch (e: unknown) {
      testResult.classList.add("err");
      testResult.textContent = `✗ ${errMsg(e)}`;
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
      name: "新しいプロバイダ",
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
    empty.appendChild(el("span", undefined, "モードがありません。「＋ モードを追加」から作成してください。"));
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
  nameInput.placeholder = "モード名";
  nameInput.addEventListener("input", () => {
    mode.name = nameInput.value;
    renderHomeModes();
    scheduleSave();
  });

  const useLlmLabel = el("label", "use-llm-row");
  const useLlm = el("input", "switch") as HTMLInputElement;
  useLlm.type = "checkbox";
  useLlm.checked = mode.use_llm;
  useLlmLabel.append(el("span", undefined, "AI整形を使う"), useLlm);

  const delBtn = el("button", "icon-btn danger", "🗑");
  delBtn.title = "削除";
  delBtn.addEventListener("click", () => {
    if (!settings) return;
    if (settings.modes.length <= 1) {
      toast("最後のモードは削除できません", true);
      return;
    }
    if (!window.confirm(`モード「${mode.name}」を削除しますか?`)) return;
    settings.modes = settings.modes.filter((m) => m.id !== mode.id);
    if (settings.active_mode_id === mode.id) {
      settings.active_mode_id = settings.modes[0]?.id ?? "";
    }
    renderModeList();
    renderHomeModes();
    scheduleSave();
  });

  if (isActive) head.appendChild(el("span", "badge accent", "アクティブ"));
  head.append(nameInput, useLlmLabel, delBtn);

  const instr = el("textarea") as HTMLTextAreaElement;
  instr.value = mode.instruction;
  instr.placeholder = "整形指示 (例: フィラーを除去し、自然な文章に整えてください)";
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
      name: "新しいモード",
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
  copyBtn.title = "コピー";
  copyBtn.addEventListener("click", async (e) => {
    e.stopPropagation();
    try {
      await invoke("copy_text", { text: entry.final_text });
      toast("コピーしました");
    } catch (err: unknown) {
      toast(`コピーに失敗しました: ${errMsg(err)}`, true);
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
        raw.appendChild(el("span", "history-raw-label", "文字起こし (生テキスト)"));
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
    empty.appendChild(el("span", undefined, "履歴はまだありません。"));
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
    if (!window.confirm("履歴をすべて削除しますか? この操作は取り消せません。")) return;
    try {
      await invoke("clear_history");
      historyEntries = [];
      renderHistoryList();
      renderHomeRecent();
      toast("履歴を削除しました");
    } catch (e: unknown) {
      toast(`削除に失敗しました: ${errMsg(e)}`, true);
    }
  });
}

// ---------------------------------------------------------------------------
// general section
// ---------------------------------------------------------------------------

function renderGeneralSection(): void {
  if (!settings) return;
  input("in-hotkey").value = settings.hotkey;
  selectEl("in-language").value = settings.language;
  selectEl("in-paste-mode").value = settings.paste_mode;
  input("in-history-limit").value = String(settings.history_limit);
  input("in-restore-clipboard").checked = settings.restore_clipboard;
  input("in-autostart").checked = settings.autostart;
}

function normalizeHotkeyKey(e: KeyboardEvent): string | null {
  const k = e.key;
  if (k === "Control" || k === "Alt" || k === "Shift" || k === "Meta") return null;
  if (k === " " || e.code === "Space") return "Space";
  if (k.length === 1) {
    const upper = k.toUpperCase();
    // use physical key for letters so IME/layout doesn't interfere
    if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
    if (/^Digit[0-9]$/.test(e.code)) return e.code.slice(5);
    return upper;
  }
  return k; // F1-F12, Enter, Tab, ArrowUp, Home, ...
}

function wireGeneralSection(): void {
  const hotkeyInput = input("in-hotkey");
  hotkeyInput.addEventListener("focus", () => {
    hotkeyInput.classList.add("capturing");
    hotkeyInput.value = "キーを押してください…";
  });
  hotkeyInput.addEventListener("blur", () => {
    hotkeyInput.classList.remove("capturing");
    if (settings) hotkeyInput.value = settings.hotkey;
  });
  hotkeyInput.addEventListener("keydown", (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (!settings) return;
    if (e.key === "Escape") {
      hotkeyInput.blur();
      return;
    }
    const mods: string[] = [];
    if (e.ctrlKey) mods.push("Ctrl");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    const key = normalizeHotkeyKey(e);
    if (!key) {
      hotkeyInput.value = mods.length > 0 ? `${mods.join("+")}+…` : "キーを押してください…";
      return;
    }
    const isFKey = /^F([1-9]|1[0-9]|2[0-4])$/.test(key);
    if (mods.length === 0 && !isFKey) {
      hotkeyInput.value = "修飾キー(Ctrl/Alt/Shift)と組み合わせてください";
      return;
    }
    const combo = [...mods, key].join("+");
    settings.hotkey = combo;
    hotkeyInput.value = combo;
    $("home-hotkey").textContent = combo;
    scheduleSave();
    hotkeyInput.blur();
  });

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

function renderSetupSection(): void {
  const s = setupStatus;
  const serverBadge = $("server-badge");
  const serverPath = $("server-path");
  const installBtn = $("btn-install-server") as HTMLButtonElement;

  if (!s) {
    serverBadge.textContent = "確認中…";
    serverBadge.className = "badge";
    serverPath.textContent = "—";
    installBtn.disabled = true;
    return;
  }

  serverBadge.textContent = s.server_installed ? "インストール済み" : "未インストール";
  serverBadge.className = `badge${s.server_installed ? " ok" : ""}`;
  serverPath.textContent = s.server_path || "—";
  installBtn.textContent = s.server_installed ? "再インストール" : "インストール";
  installBtn.disabled = activeDownloads.has(downloadKey("server", "whisper-server"));

  renderSetupModels();
}

function renderSetupModels(): void {
  const host = $("setup-model-list");
  host.textContent = "";
  const models = setupStatus?.models ?? [];
  if (models.length === 0) {
    host.appendChild(el("div", "hint", "モデル情報を取得中…"));
    return;
  }
  for (const m of models) {
    const card = el("div", "card setup-card");
    card.style.marginBottom = "0";
    const row = el("div", "setup-row");

    const info = el("div", "setup-info");
    const name = el("div", "setup-name");
    name.appendChild(document.createTextNode(`ggml-${m.name}.bin`));
    const badge = el(
      "span",
      `badge${m.installed ? " ok" : ""}`,
      m.installed ? "インストール済み" : "未インストール",
    );
    name.appendChild(badge);
    info.appendChild(name);
    row.appendChild(info);

    const right = el("div");
    right.style.display = "flex";
    right.style.alignItems = "center";
    right.appendChild(el("span", "setup-size", `${m.size_mb} MB`));
    const dlBtn = el(
      "button",
      `btn btn-sm${m.installed ? "" : " btn-primary"}`,
      m.installed ? "再ダウンロード" : "ダウンロード",
    ) as HTMLButtonElement;
    const key = downloadKey("model", m.name);
    dlBtn.disabled = activeDownloads.has(key);
    dlBtn.addEventListener("click", () => void startModelDownload(m.name));
    right.appendChild(dlBtn);
    row.appendChild(right);
    card.appendChild(row);

    // progress bar (hidden unless downloading)
    const prog = buildProgressEl(key);
    card.appendChild(prog);

    host.appendChild(card);
  }
  // container gap handles spacing
  host.style.gap = "10px";
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
    activeDownloads.delete(key);
    wrap.hidden = true;
    ($("btn-install-server") as HTMLButtonElement).disabled = false;
    toast(`サーバのインストールに失敗しました: ${errMsg(e)}`, true);
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
  renderSetupModels();
  try {
    await invoke("download_model", { model: name });
  } catch (e: unknown) {
    activeDownloads.delete(key);
    renderSetupModels();
    toast(`モデルのダウンロードに失敗しました: ${errMsg(e)}`, true);
  }
}

function onDownloadProgress(p: DownloadProgressPayload): void {
  const key = downloadKey(p.kind, p.kind === "server" ? "whisper-server" : p.name);
  if (p.done) {
    activeDownloads.delete(key);
    if (p.error) {
      toast(
        p.kind === "server"
          ? `サーバのインストールに失敗しました: ${p.error}`
          : `モデル ${p.name} のダウンロードに失敗しました: ${p.error}`,
        true,
      );
    } else {
      toast(
        p.kind === "server"
          ? "whisper-server をインストールしました"
          : `モデル ${p.name} をダウンロードしました`,
      );
    }
    if (p.kind === "server") {
      $("server-progress").hidden = true;
    }
    void refreshSetupStatus();
    return;
  }
  activeDownloads.set(key, p);
  if (p.kind === "server") {
    applyProgress($("server-progress"), p);
    ($("btn-install-server") as HTMLButtonElement).disabled = true;
  } else {
    const wrap = document.querySelector<HTMLElement>(
      `#setup-model-list .progress[data-download-key="${key}"]`,
    );
    if (wrap) {
      applyProgress(wrap, p);
    } else {
      renderSetupModels();
    }
  }
}

async function refreshSetupStatus(): Promise<void> {
  try {
    setupStatus = await invoke<SetupStatus>("setup_status");
  } catch {
    setupStatus = null;
  }
  renderSetupSection();
  renderSttModelList();
}

function wireSetupSection(): void {
  $("btn-install-server").addEventListener("click", () => void startServerDownload());
  $("btn-open-config").addEventListener("click", async () => {
    try {
      await invoke("open_config_dir");
    } catch (e: unknown) {
      toast(errMsg(e), true);
    }
  });
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
  renderSetupSection();
}

// ---------------------------------------------------------------------------
// events + init
// ---------------------------------------------------------------------------

async function setupListeners(): Promise<void> {
  await listen<StatusChangedPayload>("status-changed", (event) => {
    setStatusPill(event.payload);
  });

  await listen<Settings>("settings-changed", (event) => {
    const incoming = JSON.stringify(event.payload);
    // Ignore echoes of our own save, and no-op updates. Keep the current
    // settings object so live input closures (provider/mode cards) stay valid.
    if (incoming === lastSavedJson || incoming === JSON.stringify(settings)) {
      return;
    }
    settings = event.payload;
    lastSavedJson = incoming;
    renderAll();
  });

  await listen<null>("history-updated", () => {
    void refreshHistory();
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
  wireSetupSection();

  $("btn-record-test").addEventListener("click", async () => {
    try {
      await invoke("toggle_recording");
    } catch (e: unknown) {
      toast(errMsg(e), true);
    }
  });

  $("btn-quit").addEventListener("click", async () => {
    if (!window.confirm("fire-speak を終了しますか? 常駐が解除され、ホットキーも無効になります。")) return;
    try {
      await invoke("quit_app");
    } catch (e: unknown) {
      toast(errMsg(e), true);
    }
  });

  await setupListeners();

  try {
    settings = await invoke<Settings>("get_settings");
    lastSavedJson = JSON.stringify(settings);
  } catch (e: unknown) {
    toast(`設定の読み込みに失敗しました: ${errMsg(e)}`, true);
  }

  try {
    const status = await invoke<string>("get_status");
    setStatusPill({ status: status as AppStatus });
  } catch {
    setStatusPill({ status: "idle" });
  }

  renderAll();
  void refreshHistory();
  void refreshSetupStatus();
}

window.addEventListener("DOMContentLoaded", () => {
  void init();
});
