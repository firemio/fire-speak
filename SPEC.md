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
  "ui_lang": "",
  "update": { "auto_check": true, "owner": "firemio", "repo": "fire-speak" },
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
- `ui_lang`: UI表示言語コード(下記12種)または `""` = 自動。`""` のとき settings::load 後にOSロケールから解決した値を**メモリ上だけ**セット(永続化しない)。ユーザーが選択したら実コードを保存
- `update`: アップデート確認設定。`owner`/`repo` はGitHubリポジトリ
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

---

# v0.2 追加仕様: 多言語UI (i18n) + アップデート確認

## 対応言語 (12)

| code | 表示名(セレクタはこの原語表記) |
|---|---|
| ja | 日本語 |
| en | English |
| zh-CN | 简体中文 |
| zh-TW | 繁體中文 |
| ko | 한국어 |
| es | Español |
| fr | Français |
| de | Deutsch |
| pt-BR | Português (Brasil) |
| ru | Русский |
| vi | Tiếng Việt |
| id | Bahasa Indonesia |

OSロケール解決(Rust `sys-locale`): 完全一致 → `zh-Hant*`/`zh-TW`/`zh-HK`→`zh-TW`、`zh*`→`zh-CN`、それ以外は先頭2文字一致(`ja-JP`→`ja`)、該当なし→`en`。

## i18n アーキテクチャ

- ロケール辞書: `src/locales/{code}.json`(フラットな `"key": "文字列"` マップ。プレースホルダは `{0}`, `{1}`)。**ja.json がマスター**(全キーの源泉)、全ロケールは ja と完全同一のキー集合を持つこと。
- `src/i18n.ts`: 12ロケールを静的import。`setLang(code)`, `t(key, ...params)`, `tMsg(raw)` をexport。main.ts / overlay.ts 双方から使用。
- 静的HTML(index.html / overlay.html)のテキストは `data-i18n="key"` 属性(placeholder/titleは `data-i18n-placeholder` 等)とし、起動時と言語変更時に一括適用。`<html lang>` も更新。
- 言語変更(設定「一般」のセレクタ)は保存後 **その場で再描画**(再起動不要)。overlay は settings-changed で追随。トレイはRust側で再構築。
- デフォルトモード(初回起動時生成)の name / instruction は初回解決言語で生成(バックエンドが12言語分を内蔵)。既存ユーザーのモードは変更しない。

## バックエンド発の文言 = メッセージキー方式

status-changed の `message`、コマンドの `Err(String)`、`get_startup_error` の値は、**生の文章ではなく下記の閉じたキー集合**を返す。パラメータは `|` 区切りで付加(例: `"ERR_PORT_IN_USE|8178"`)。フロントの `tMsg(raw)` が先頭キーを辞書(`msg.` プレフィクス、例 `msg.ERR_PORT_IN_USE`)で引き、`{0}`,`{1}` にパラメータを埋める。辞書に無い文字列はそのまま表示(フォールバック)。

キー一覧(これ以外を発明しない。合わないものは ERR_INTERNAL|{detail} に寄せる):

```
ERR_NO_SPEECH                  音声を認識できなかった
ERR_SETUP_REQUIRED             ローカルSTT未セットアップ
ERR_PORT_IN_USE|{port}
ERR_HOTKEY_PARSE|{combo}
ERR_HOTKEY_REGISTER|{combo}    他アプリが使用中等で登録失敗
ERR_STARTUP_HOTKEY|{detail}    起動時ホットキー登録失敗(detailは上記キーの訳出済みでなく生キー連結でよい)
ERR_NO_MIC                     入力デバイスなし
ERR_MIC_INIT|{detail}
ERR_STT_HTTP|{status}
ERR_STT_NETWORK|{detail}
ERR_LLM_HTTP|{status}
ERR_LLM_NETWORK|{detail}
WARN_LLM_FALLBACK              LLM失敗→生テキストで続行
ERR_DOWNLOAD_FAILED|{detail}
ERR_DOWNLOAD_INCOMPLETE
ERR_SERVER_START|{detail}
ERR_SERVER_DIED                whisper-server即死
ERR_ZIP|{detail}
ERR_NO_SERVER_ASSET            リリースにwin-x64アセットなし/展開後exeなし
ERR_SAVE_SETTINGS|{detail}
ERR_CLIPBOARD|{detail}
ERR_PASTE|{detail}
ERR_FILE_IO|{detail}
ERR_OPEN_FOLDER|{detail}
ERR_UPDATE_CHECK|{detail}
ERR_INTERNAL|{detail}
OK_STT                         STTテスト成功
```

例外(キー化しない): `test_llm` のOk値(モデルの生応答)、履歴/転写テキスト、`get_status` のstatus値。

トレイのラベル(「設定を開く」「終了」相当)はRust内蔵の12言語テーブルで `ui_lang` に追随。モード名はユーザーデータなのでそのまま。

## アップデート確認

GitHub Releases の最新版と現行バージョンを比較する方式(自動更新・署名なし。ダウンロードはブラウザで開く)。

### 新コマンド

| コマンド | 引数 | 戻り値 |
|---|---|---|
| `check_update` | — | `UpdateInfo`。ネットワーク失敗等は `Err("ERR_UPDATE_CHECK\|{detail}")` |
| `open_url` | `{ url: string }` | `()` — 既定ブラウザで開く。`https://` 以外は Err |
| `get_ui_lang` | — | `string` — 解決済みUI言語コード。`ui_lang` 設定が12種のいずれかならそれ、`""`(自動)/不明値ならOSロケール解決。**get_settings の ui_lang は永続値のまま**(`""` = 自動)で、解決はこのコマンドで行う。セレクタは先頭に「自動」(`""`)の選択肢を持つ |

```ts
type UpdateInfo = {
  current: string;          // 例 "0.2.0" (tauriのapp version)
  latest: string;           // 例 "0.3.1" (タグ先頭のv除去)
  update_available: boolean; // semver的比較 latest > current
  url: string;              // setup.exeアセットのbrowser_download_url、無ければreleaseのhtml_url
  notes: string;            // リリースノート(本文、最大4000文字に切詰め)
};
```

- 実装: `GET https://api.github.com/repos/{owner}/{repo}/releases/latest`(User-Agent必須、タイムアウト15秒)。タグ `vX.Y.Z` / `X.Y.Z` を数値比較。
- フロント: サイドバーに「アップデート」セクション新設 — 現行バージョン(@tauri-apps/api/app getVersion)、「更新を確認」ボタン、結果表示(新版あり: バージョン+ノート+「ダウンロードページを開く」)、auto_checkトグル、owner/repo入力(詳細折りたたみ)。
- 自動チェック: main初期化時に `update.auto_check` なら silent 実行(失敗は無視)。新版ありならホーム上部にバナー(バージョン+開くボタン+閉じる)。
- バージョンは tauri.conf.json / package.json の `version`(単一ソースはtauri.conf.json、表示はgetVersion)。

## ファイル所有権 (v0.2作業)

- バックエンド: `src-tauri/src/**`, `src-tauri/Cargo.toml`
- フロントエンド(コア): `src/**`(locales含む), `index.html`, `overlay.html`
- 翻訳担当(言語別): `src/locales/{自分のcode}.json` **のみ**
- 変更禁止: `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`, `package.json`(バージョンは統合者が管理)

---

# v0.3 追加仕様: 修飾キー単体ホットキー（既定: 右Alt）

## 設定値

- `settings.hotkey` は従来のコンボ(`"Ctrl+Alt+Space"`)に加え、**特殊トークン** `RAlt | LAlt | RCtrl | LCtrl | RShift | LShift` を許可。
- **既定値を `"RAlt"`(右Alt単体)に変更**(新規作成時のみ。既存settings.jsonは変更しない)。

## バックエンド (hook.rs 新設)

- 特殊トークン時は global-shortcut プラグインを使わず、**WH_KEYBOARD_LL 低レベルフック**で検知:
  - 起動時に専用スレッド(メッセージポンプ付き)でフックを1回インストール。監視VKは `AtomicU32`(0=無効)。RAlt=0xA5, LAlt=0xA4, RCtrl=0xA3, LCtrl=0xA2, RShift=0xA1, LShift=0xA0。
  - 対象VKの keydown で `pipeline::toggle`(コールバック内は最小処理、toggleは別スレッドへ)。
  - `LLKHF_INJECTED` は無視(enigo等の合成入力に反応しない)。オートリピート抑止(down状態を保持し、up→downの遷移のみ発火)。
  - 対象キーの down/up は **swallow**(return 1)し、他アプリへのAltメニュー等の副作用を防ぐ。downをswallowしたら対応するupもswallowする。
- コンボ⇔特殊トークン切替: `register_hotkey` で「特殊トークンならフックatomicをセットしプラグインを全解除」「コンボならatomic=0にしプラグイン登録」。検証順序は従来通り(**先に検証・成功時のみ永続化**)。特殊トークンはフック有効化が失敗した場合のみ `ERR_HOTKEY_REGISTER|{token}`。
- 新コマンド `set_hotkey_suspended` `{ suspended: bool }` → `()`:
  - true の間、**ホットキー経路**(フック・プラグインの両ハンドラ先頭でAtomicBoolチェック)での toggle を無効化。フックは swallow もしない(素通し=キャプチャ欄にキーが届く)。
  - UIの録音ボタン(`toggle_recording`)には影響しない。

## フロントエンド

- キャプチャ欄: focus で `set_hotkey_suspended(true)`、blur で `false`(失敗は無視)。
- **修飾キー単体タップ検出**: keydown が修飾キーのみなら保留表示し、対応する keyup までに他キーが来なければ e.code から確定(AltRight→RAlt, AltLeft→LAlt, ControlRight→RCtrl, ControlLeft→LCtrl, ShiftRight→RShift, ShiftLeft→LShift)。途中で他キーが来たら従来のコンボ処理。確定は既存の applyHotkey 経由(バックエンド検証・失敗時ロールバック)。
- **表示**: `displayHotkey(hotkey)` — 特殊トークンは locale キー `hotkey.RAlt` 等で表示(ja「右Alt」/ en「Right Alt」等)。ホームと一般タブの表示・トースト等ホットキーを表示する全箇所で使用。
- locale キー追加(**12言語全ファイル、キーパリティ維持**): `hotkey.RAlt` `hotkey.LAlt` `hotkey.RCtrl` `hotkey.LCtrl` `hotkey.RShift` `hotkey.LShift` `general.hotkeyHint`(修飾キー単体も設定可能という短い説明)。

## 注意

- windows-sys の features に `Win32_UI_WindowsAndMessaging` `Win32_Foundation` 等フックに必要なものを追加。
- 貼り付け前の修飾キー解放待ち(paste.rs)は従来通り(RAltを押したまま確定した場合は物理解放を待つ)。
