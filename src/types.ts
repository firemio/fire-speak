// Shared types mirroring SPEC.md (backend contract). Field names must match exactly.

export type SttEngine = "local" | "cloud";
export type PasteMode = "paste" | "clipboard";
export type ProviderKind = "anthropic" | "openai";
/** v0.3: "hold" = record while the hotkey is held (default), "toggle" = press to start/stop. */
export type HotkeyMode = "hold" | "toggle";

export interface LocalSttSettings {
  server_port: number;
  model_path: string;
  server_path: string;
  threads: number;
}

export interface CloudSttSettings {
  base_url: string;
  api_key: string;
  model: string;
}

export interface SttSettings {
  engine: SttEngine;
  local: LocalSttSettings;
  cloud: CloudSttSettings;
}

export interface LlmProvider {
  id: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  api_key: string;
  model: string;
}

export interface LlmSettings {
  active_provider_id: string;
  providers: LlmProvider[];
}

export interface Mode {
  id: string;
  name: string;
  instruction: string;
  use_llm: boolean;
}

export interface UpdateSettings {
  auto_check: boolean;
  owner: string;
  repo: string;
}

export interface Settings {
  hotkey: string;
  hotkey_mode: HotkeyMode;
  language: string;
  /** UI display language code (one of the 12 SPEC codes). The backend
   * resolves "" (auto) to a concrete code before the frontend sees it. */
  ui_lang: string;
  update: UpdateSettings;
  active_mode_id: string;
  paste_mode: PasteMode;
  restore_clipboard: boolean;
  autostart: boolean;
  history_limit: number;
  stt: SttSettings;
  llm: LlmSettings;
  modes: Mode[];
}

export interface HistoryEntry {
  id: string;
  timestamp: number; // unix ms
  mode_id: string;
  raw_text: string;
  final_text: string;
}

export interface SetupModelInfo {
  name: string;
  installed: boolean;
  size_mb: number;
}

export interface SetupStatus {
  server_installed: boolean;
  server_path: string;
  model_installed: boolean;
  model_path: string;
  /** Absolute path of the managed models directory ({app_data}/models). */
  models_dir: string;
  models: SetupModelInfo[];
}

export type AppStatus =
  | "idle"
  | "recording"
  | "transcribing"
  | "polishing"
  | "done"
  | "error";

export interface StatusChangedPayload {
  status: AppStatus;
  message?: string;
}

export interface LevelPayload {
  rms: number; // 0.0 - 1.0
}

export interface ResultPayload {
  raw_text: string;
  final_text: string;
}

export interface UpdateInfo {
  current: string; // e.g. "0.2.0" (tauri app version)
  latest: string; // e.g. "0.3.1" (leading "v" stripped from the tag)
  update_available: boolean; // semver-ish comparison latest > current
  url: string; // setup.exe asset browser_download_url, else the release html_url
  notes: string; // release notes body, truncated to 4000 chars (UNTRUSTED — render as textContent only)
}

export interface DownloadProgressPayload {
  kind: "server" | "model";
  name: string;
  downloaded: number;
  total: number;
  done: boolean;
  error?: string;
}
