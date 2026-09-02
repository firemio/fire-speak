// Shared types mirroring SPEC.md (backend contract). Field names must match exactly.

export type SttEngine = "local" | "cloud";
export type PasteMode = "paste" | "clipboard";
export type ProviderKind = "anthropic" | "openai";

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

export interface Settings {
  hotkey: string;
  language: string;
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

export interface DownloadProgressPayload {
  kind: "server" | "model";
  name: string;
  downloaded: number;
  total: number;
  done: boolean;
  error?: string;
}
