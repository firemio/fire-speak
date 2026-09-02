# fire-speak — 仕様契約書 (SPEC)

Genspark Speakly 相当の常駐型AI音声入力アプリ。Windows 11 / Tauri v2。
グローバルホットキー → 録音 → 文字起こし(STT) → モード別AI整形(LLM) → アクティブアプリへ自動貼り付け。

この文書は **バックエンド(Rust)とフロントエンド(TS/HTML)の契約**。コマンド名・イベント名・JSONシェイプは一字一句この通りに実装すること。

## ファイル所有権

- バックエンド担当: `src-tauri/src/**`, `src-tauri/Cargo.toml`
- フロントエンド担当: `src/**`, `index.html`, `overlay.html`, `vite.config.ts`, `package.json`(必要時のみ)
- 変更禁止(両者): `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`

## ウィンドウ構成 (tauri.conf.json で定義済み・変更禁止)

- `main`: 設定UI。980x720。閉じるボタン → **終了せず hide**(Rust側で `on_window_event` CloseRequested を prevent し hide)。トレイから再表示。
- `overlay`: 録音HUD。380x110、透過・枠なし・最前面・タスクバー非表示・**focus:false**(アクティブアプリのフォーカスを奪わない)。表示時に画面下部中央へ配置(Rust側で position 計算)。通常は非表示。

## 設定スキーマ (settings.json — Rust `Settings` 構造体 / TSでも同名フィールド)

保存先: `{app_config_dir}/settings.json` (例 `%APPDATA%/jp.firemio.fire-speak/settings.json`)。
serde は全フィールド `#[serde(default = ...)]` で欠損に耐えること。

```json
{
  "hotkey": "Ctrl+Alt+Space",
  "language": "auto",
  "active_mode_id": "polish",
  "paste_mode": "paste",
  "restore_clipboard": true,
  "autostart": false,
  "history_limit": 50,
  "stt": {
    "engine": "local",
    "local": { "server_port": 8178, "model_path": "", "server_path": "", "threads": 4 },
    "cloud": { "base_url": "https://api.openai.com/v1", "api_key": "", "model": "whisper-1" }
  },
  "llm": {
    "active_provider_id": "anthropic",
    "providers": [
      { "id": "anthropic", "name": "Claude (Anthropic)", "kind": "anthropic",
        "base_url": "https://api.anthropic.com", "api_key": "", "model": "claude-haiku-4-5" },
      { "id": "laguna", "name": "Laguna S 2.1 (OpenRouter free)", "kind": "openai",
        "base_url": "https://openrouter.ai/api/v1", "api_key": "", "model": "poolside/laguna-s-2.1:free" }
    ]
  },
  "modes": [ { "id": "polish", "name": "整形", "instruction": "…", "use_llm": true }, … ]
}
```

- `language`: `"auto" | "ja" | "en" | …` (whisperの言語ヒント。auto時はパラメータ送らない/`auto`)
- `paste_mode`: `"paste"`(クリップボード経由でCtrl+V自動送信) | `"clipboard"`(コピーのみ)
- `stt.engine`: `"local" | "cloud"`
- `stt.local.server_path` / `model_path`: 空文字 = 管理ディレクトリの既定を使う
- `llm.providers[].kind`: `"anthropic" | "openai"`(openai = OpenAI互換chat/completions。OpenRouter/Groq/Ollama等も同じ)

### デフォルトモード (初回起動時に生成)

| id | name | use_llm | instruction |
|---|---|---|---|
| polish | 整形 | true | フィラー(「えー」「あの」「um」等)と言い直しを除去し、句読点・改行を整えて自然な文章にしてください。内容・意味は変えないでください。話者が使った言語のまま出力してください。 |
| raw | そのまま | false | (空) |
| to_en | 英語に翻訳 | true | 内容を自然で流暢な英語に翻訳してください。フィラーは除去してください。 |
| to_ja | 日本語に翻訳 | true | 内容を自然な日本語に翻訳してください。フィラーは除去してください。 |
| terminal | ターミナルコマンド | true | 発話内容を Windows PowerShell で実行可能なコマンドに変換してください。コマンドのみを出力し、説明やコードフェンスは付けないでください。 |
| business | ビジネス文体 | true | 内容を丁寧なビジネス日本語(です・ます調)に書き直してください。フィラーは除去し、簡潔で礼儀正しい文章にしてください。 |

## Tauriコマンド (invoke) — 名前・引数・戻り値は厳守

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `get_settings` | — | `Settings` |
| `save_settings` | `{ settings: Settings }` | `()` — 保存+ホットキー再登録+autostart適用+ローカルSTT設定変更時はサーバ再起動。**ホットキーが解釈不能/登録失敗の場合は何も永続化せず Err**(既存ホットキーは維持)。フロントは Err 時に旧値へ戻す |
| `toggle_recording` | — | `()` — ホットキーと同じ動作 |
| `cancel_recording` | — | `()` — 録音/処理を破棄しoverlayを隠す |
| `get_status` | — | `string` (下記status値) |
| `get_startup_error` | — | `string \| null` — 起動時(リスナー登録前)に発生したエラーを1回だけ返す(取得で消費)。フロントはinitで取得しトースト表示 |
| `set_active_mode` | `{ modeId: string }` | `()` — 保存+`settings-changed`発火+トレイメニュー更新 |
| `get_history` | — | `HistoryEntry[]` (新しい順) |
| `clear_history` | — | `()` |
| `copy_text` | `{ text: string }` | `()` — クリップボードへコピー |
| `test_stt` | — | `string` (成功メッセージ。失敗は Err(文字列)) |
| `test_llm` | `{ providerId: string }` | `string` (モデルの応答テキスト。失敗は Err) |
| `setup_status` | — | `SetupStatus` |
| `download_whisper_server` | — | `()` — 進捗は`download-progress`イベント |
| `download_model` | `{ model: string }` | `()` — model: `"tiny"\|"base"\|"small"\|"medium"\|"large-v3-turbo"` |
| `open_config_dir` | — | `()` — エクスプローラで設定フォルダを開く |
| `quit_app` | — | `()` |

注: Tauri v2 は Rust側 snake_case 引数を **JS側 camelCase** で受ける(`modeId` → `mode_id`)。

```ts
type HistoryEntry = { id: string; timestamp: number /*unix ms*/; mode_id: string; raw_text: string; final_text: string };
type SetupStatus = {
  server_installed: boolean; server_path: string;
  model_installed: boolean; model_path: string;
  models_dir: string; // 管理モデルディレクトリ({app_data}/models)の絶対パス
  models: { name: string; installed: boolean; size_mb: number }[];
};
```

## イベント (backend → frontend, `listen`)

| イベント名 | payload |
|---|---|
| `status-changed` | `{ status: "idle"\|"recording"\|"transcribing"\|"polishing"\|"done"\|"error", message?: string }` |
| `level` | `{ rms: number }` 0.0〜1.0、録音中 約60ms間隔 (HUDの波形アニメ用) |
| `result` | `{ raw_text: string, final_text: string }` |
| `download-progress` | `{ kind: "server"\|"model", name: string, downloaded: number, total: number, done: boolean, error?: string }` |
| `history-updated` | `null` |
| `settings-changed` | `Settings` (トレイ等から変更時。設定画面は再描画) |

## パイプライン動作 (バックエンド)

1. ホットキー(既定 `Ctrl+Alt+Space`)トグル: idle→録音開始 / recording→停止して処理開始。処理中(transcribing/polishing)の押下は無視。
2. 録音開始: overlay表示(下部中央, フォーカス奪わない)、`status-changed{recording}`、cpalで既定マイクから取得。**16kHz mono 16bit WAV**に変換(デバイスレートからのダウンサンプルは線形補間で可)。最大5分で自動停止。`level`イベント送出。
3. 停止: `status-changed{transcribing}` → STT実行。
   - local: 管理中の whisper-server へ `POST http://127.0.0.1:{port}/inference` multipart (`file`=wav, `response_format=json`, `temperature=0.0`, language≠auto なら `language`)。レスポンス `{"text": "..."}`。サーバ未起動なら起動して待機(後述)。
   - cloud: `POST {base_url}/audio/transcriptions` Bearer, multipart (`file`, `model`, language≠auto なら `language`)。
4. 空文字/空白のみなら `status-changed{error, "音声を認識できませんでした"}` → 2.5秒後overlay隠す。
5. アクティブモードの `use_llm` が true かつプロバイダのAPIキーあり → `status-changed{polishing}` → LLM整形。**LLM失敗時は raw_text をそのまま使い、続行**(message で警告)。
6. 貼り付け: `paste_mode=="paste"` → 旧クリップボード退避(テキストのみ) → 新テキストset → 60ms待ち → enigoで Ctrl+V → 400ms待ち → `restore_clipboard` なら復元。`"clipboard"` → コピーのみ。
7. `result`発火、履歴保存(`history_limit`件まで、`{app_config_dir}/history.json`)、`history-updated`発火、`status-changed{done}` → 1.2秒後 overlay 隠して idle へ。
8. エラー時: `status-changed{error, message}` → 2.5秒後 overlay 隠して idle。
9. 録音中の再度キャンセル: `cancel_recording` または overlay の✕ボタン(frontendから invoke)。

## LLM整形リクエスト

system prompt (共通・日本語):
```
あなたは音声入力の後処理エンジンです。ユーザーが口述したテキストが与えられます。以下の指示に従って処理し、処理後のテキストだけを出力してください。前置き・引用符・説明・コードフェンスは一切付けないでください。

指示: {mode.instruction}
```
user メッセージ = STTの生テキスト。

- kind=`anthropic`: `POST {base_url}/v1/messages`, headers `x-api-key`, `anthropic-version: 2023-06-01`, body `{model, max_tokens: 4096, system, messages:[{role:"user",content: raw}]}` → `content[0].text`
- kind=`openai`: `POST {base_url}/chat/completions`, `Authorization: Bearer`, body `{model, messages:[{role:"system"...},{role:"user"...}]}` → `choices[0].message.content`

タイムアウト: STT 120秒 / LLM 60秒。

## ローカルSTT管理 (setup.rs)

- 管理ディレクトリ: `{app_data_dir}/bin/`(whisper-server) と `{app_data_dir}/models/`
- **whisper-server ダウンロード**: GitHub API `https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest` → assets から名前に `bin-x64` を含む `.zip`(無ければ `(?i)win.*x64.*\.zip`)を選び、User-Agent必須でDL → zip展開 → `*server*.exe` を探して `server_path` 確定(dll類も同フォルダに展開)。進捗を `download-progress` で送出。
- **モデルDL**: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin` (リダイレクト追従)。サイズ目安: tiny=78MB, base=148MB, small=488MB, medium=1533MB, large-v3-turbo=1624MB。
- **サーバ起動**: 初回transcribe時または設定変更時に spawn: `whisper-server.exe -m {model} --port {port} --host 127.0.0.1 -t {threads}`。Windowsでは `creation_flags(0x08000000)` (CREATE_NO_WINDOW)。起動後TCP接続でヘルスチェック(最長20秒リトライ)。アプリ終了時・設定変更時にkill。
- モデル/サーバ未インストールで engine=local のままtranscribe要求 → エラーメッセージ「セットアップ画面からWhisperサーバとモデルをインストールしてください」。

## トレイ (Rust)

- アイコン常駐。左クリック → main表示+フォーカス。
- メニュー: モード一覧(ラジオ的にチェック、クリックで `set_active_mode` 相当) / 区切り / 「設定を開く」 / 「終了」。
- モード変更・settings保存時にメニュー再構築。

## フロントエンドUI要件

### index.html (設定画面) — 左サイドバー + コンテンツ

セクション:
1. **ホーム**: 現在の状態、アクティブモード切替(大きめのカード/セグメント)、ホットキー表示、「録音テスト」ボタン(toggle_recording)、直近履歴3件
2. **音声認識**: engine切替(ローカル/クラウド)。ローカル: モデル選択+DL状況、詳細(ポート/スレッド/パス上書き)。クラウド: base_url/api_key/model。テストボタン(test_stt)
3. **AI整形**: プロバイダ一覧(追加/編集/削除)、kind選択(anthropic/openai互換)、base_url/model/api_key(パスワード型・表示切替)、アクティブ選択、テストボタン(test_llm)
4. **モード**: 一覧+追加/編集/削除(name, instruction, use_llm)。デフォルトモードも編集可。ドラッグ並べ替えは不要
5. **履歴**: 一覧(時刻・モード・整形後テキスト、クリックで生テキストも展開)、コピー(copy_text)、全消去
6. **一般**: ホットキー変更(キー押下キャプチャ式: Ctrl/Alt/Shift+キー → "Ctrl+Alt+Space"形式の文字列)、言語(auto/ja/en)、paste_mode、restore_clipboard、autostart
7. **セットアップ**: setup_status表示、whisper-serverインストールボタン、モデル一覧とDLボタン、進捗バー(download-progress)、open_config_dir

- 保存は明示ボタンでなく **変更時に自動保存**(debounce 500ms で save_settings)。ただしAPIキー入力はblurで保存。
- `settings-changed` 受信で再描画(無限ループ注意: 自分のsave由来は無視してよい)。

### overlay.html (HUD)

- 角丸ダーク半透明カード。中央にマイク波形(levelイベントでバー高さアニメ、10〜16本)。
- status表示: recording「話してください… (ホットキーで確定)」/ transcribing「文字起こし中…」/ polishing「AI整形中…」/ done「✓ 貼り付けました」/ error はメッセージ赤表示。
- 右上に小さな✕(cancel_recording を invoke)。アクティブモード名を小さく表示。
- `-webkit-app-region` は使わない(クリック領域が必要)。背景は `background: transparent`、bodyにカードを描く。

### デザイン方向

- ダークテーマ基調 (#0f1115 系)、アクセントは炎オレンジ (#ff6b35 → #ff9558 グラデ)。名前は「fire-speak 🔥」。
- モダンで簡潔、影とradius(12px)、システムフォント(`"Segoe UI", "Hiragino Sans", "Noto Sans JP", sans-serif`)。UI文言は日本語。
- フレームワーク不使用(vanilla TS)。`src/main.ts`(設定画面) と `src/overlay.ts`(HUD) を分離。CSSは `src/styles.css` と `src/overlay.css`。

### vite.config.ts

マルチページ設定必須:
```ts
build: { rollupOptions: { input: { main: "index.html", overlay: "overlay.html" } } }
```
(既存のtauri用設定 — clearScreen/server設定 — は残すこと)

## Rustモジュール構成 (推奨)

- `lib.rs` — setup(トレイ/ホットキー/プラグイン/State)、コマンド登録、ウィンドウイベント
- `settings.rs` — Settings型・load/save・デフォルト
- `audio.rs` — cpal録音(開始/停止/レベル送出/WAV生成)
- `stt.rs` — local/cloud transcribe
- `llm.rs` — anthropic/openai polish
- `paste.rs` — クリップボード+Ctrl+V (enigo/arboard)
- `pipeline.rs` — 状態機械(idle/recording/…)とフロー統括
- `setup.rs` — whisper-server/モデルのDL・展開・プロセス管理
- `history.rs` — 履歴の読み書き

依存追加は `cargo add` で最新安定版を使用: `tauri-plugin-global-shortcut tauri-plugin-single-instance tauri-plugin-autostart cpal hound reqwest(rustls-tls,json,multipart,stream) tokio(full) enigo arboard dirs zip futures-util uuid(v4)`。
tauri本体は `features = ["tray-icon", "image-png"]` を有効化。

## 品質要件

- APIキーをログに出さない。`println!`/`eprintln!` はエラー診断最小限。
- 全コマンドは `Result<T, String>` でエラーを日本語メッセージ化。panicさせない(`unwrap`は初期化時のみ可)。
- 録音デバイスなし・サーバ起動失敗・API 4xx/5xx はすべてoverlayにエラー表示で回復(アプリは落ちない)。
- `cargo build` と `npm run build`(tsc) が警告はあってもエラー0で通ること。
