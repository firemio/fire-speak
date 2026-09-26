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
  "autostart": true,
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

- `language`: whisper言語コード(`"ja" | "en" | "zh" | "ko" | "es" | "fr" | "de" | "pt" | "ru" | "vi" | "id"`)または `"auto"`。auto時はパラメータを送らない。**初回起動時の値は OS ロケールから決定**(v0.5、`locale::default_stt_language`)。serde の静的既定は後方互換のため `"auto"` のまま
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
- **サーバ起動**: 初回transcribe時または設定変更時に spawn: `whisper-server.exe -m {model} --port {port} --host 127.0.0.1 -t {threads}`。Windowsでは `creation_flags(0x08000000)` (CREATE_NO_WINDOW)。起動後TCP接続でヘルスチェック(最長20秒リトライ、v0.9 で 60 秒)。アプリ終了時・設定変更時にkill。
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
6. **一般**: ホットキー変更(キー押下キャプチャ式: Ctrl/Alt/Shift+キー → "Ctrl+Alt+Space"形式の文字列)、認識言語(自動判定 + 11言語、v0.5)、paste_mode、restore_clipboard、autostart、live_caption(v0.5)
7. ~~**セットアップ**~~ (v0.6 で **音声認識** に統合: whisper-server カードとモデル一覧+DLボタン+進捗バー、open_config_dir はローカルパネル内)

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

# v0.3 追加仕様: 押しっぱなし録音（プッシュ・トゥ・トーク）+ 修飾キー単体ホットキー（既定: 右Alt押しっぱなし）

Genspark Speak と同じ「**右Altを押している間だけ録音、離すと確定**」を既定動作にする。

## 設定値

- `settings.hotkey` は従来のコンボ(`"Ctrl+Alt+Space"`)に加え、**特殊トークン** `RAlt | LAlt | RCtrl | LCtrl | RShift | LShift` を許可。
- **既定値を `"RAlt"`(右Alt単体)に変更**(新規作成時のみ。既存settings.jsonは変更しない)。
- `settings.hotkey_mode` を追加: `"hold"`(押している間録音・既定) | `"toggle"`(押すたび開始/確定)。serde default = `"hold"`(既存ファイルは欠損→hold)。

## ホットキー動作モード (pipeline)

- 共通エントリを2つに分離: `pipeline::hotkey_pressed(app)` / `pipeline::hotkey_released(app)`。プラグイン(`ShortcutState::Pressed/Released`)とフック(keydown/keyup)の**両方**がこの2つを呼ぶ。suspend中はどちらも無視。
- **toggle モード**: Pressed = 従来の toggle()(idle→開始 / recording→確定)。Released は無視。
- **hold モード(既定)**:
  - Pressed: idle→録音開始+押下時刻記録。recording(タップロック中)→確定(stop_and_process)。処理中→無視。
  - Released: recording かつ押下から **400ms以上** → 確定。**400ms未満** → 何もしない(録音継続=**タップロック**。誤タップによる0.2秒録音を防ぎ、短押し=トグルとしても使える)。
  - オートリピート対策: held フラグ(AtomicBool)。Pressed は held false→true の遷移のみ処理、Released で held=false に戻す。

## バックエンド (hook.rs 新設)

- 特殊トークン時は global-shortcut プラグインを使わず、**WH_KEYBOARD_LL 低レベルフック**で検知:
  - 起動時に専用スレッド(メッセージポンプ付き)でフックを1回インストール。監視VKは `AtomicU32`(0=無効)。RAlt=0xA5, LAlt=0xA4, RCtrl=0xA3, LCtrl=0xA2, RShift=0xA1, LShift=0xA0。
  - 対象VKの keydown→`hotkey_pressed` / keyup→`hotkey_released`(コールバック内は最小処理: atomic判定+swallow判断のみ。pressed/released 呼び出しは別スレッドへ。**LLフックのコールバックが遅いと全システムのキー入力を遅延させるため、割り当て・ロック待ち・I/Oを一切しないこと**)。
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
- **動作モードUI**: 一般タブに `hotkey_mode` のラジオ/セグメント(「押している間だけ録音(推奨)」「押すたびに開始/停止」)。変更は通常のdebounce保存。
- **overlay**: hold モード時の録音中テキストは `overlay.recordingHold`(「話してください…(離すと確定)」)、toggle モード時は従来の `overlay.recording`。overlayは settings.hotkey_mode で分岐(settings-changedで追随)。
- locale キー追加(**12言語全ファイル、キーパリティ維持**): `hotkey.RAlt` `hotkey.LAlt` `hotkey.RCtrl` `hotkey.LCtrl` `hotkey.RShift` `hotkey.LShift` `general.hotkeyHint`(修飾キー単体も設定可能という短い説明) `general.hotkeyMode` `general.hotkeyModeHold` `general.hotkeyModeToggle` `overlay.recordingHold`。翻訳は全12言語とも自然なUI文で書くこと。

## 注意

- windows-sys の features に `Win32_UI_WindowsAndMessaging` `Win32_Foundation` 等フックに必要なものを追加。
- 貼り付け前の修飾キー解放待ち(paste.rs)は従来通り(RAltを押したまま確定した場合は物理解放を待つ)。

## v0.3.1 修正仕様(レビュー確定所見の修正 — この節が上の記述と矛盾する場合はこちらが優先)

### フック状態機械の対称化 (hook.rs)

- `KEY_DOWN` を廃止し、**`SWALLOWED_VK`(AtomicU32, 0=なし)を唯一の押下ラッチ**とする:
  - keydown: `vk == SWALLOWED_VK` なら**無条件でswallow**(タイプマチックリピート。watched/suspend状態に関わらず)。それ以外で `vk == watched && !suspended && !injected && AltGrシーケンスでない` なら `SWALLOWED_VK=vk` → pressed をディスパッチ → swallow。どちらでもなければ素通し。
  - keyup: `vk == SWALLOWED_VK` なら `SWALLOWED_VK=0` → **released を suspend 状態に関わらず必ずディスパッチ** → swallow。それ以外は素通し(素通しdownのupは素通し=対称)。
- **ディスパッチは単一スレッド**: install時に `mpsc::channel<bool>` + 常駐ディスパッチャスレッドを作り、コールバックは `send(pressed)` のみ(O(1)、順序保証。イベント毎の thread::spawn を廃止)。
- **AltGrパススルー**: 非injectedの LCtrl(0xA2) down/up で **scanCode == 0x21D**(AltGr偽Ctrl)を観測したら直近タイムスタンプを記録。RAlt イベントが同一 time で来たら AltGr 由来としてラッチ・ディスパッチ・swallowを一切せず素通し。

### AltGr配列対策 (locale.rs / settings.rs / lib.rs)

- `locale::layout_uses_altgr() -> bool`: `VkKeyScanExW` で 0x20..0x100 の文字を現在のHKLで走査し、修飾バイトに CTRL|ALT(0x06) を要求する文字が1つでもあれば true。
- 初回起動時の既定 hotkey: AltGr配列なら `"Ctrl+Alt+Space"`、それ以外は `"RAlt"`(settings::load の first_run で決定)。
- `register_hotkey`: `RAlt` 指定かつ AltGr配列 → `ERR_HOTKEY_REGISTER|RAlt` (UIにトースト表示され、旧ホットキー維持・非永続化の既存フローに乗る)。

### その他バックエンド

- `set_hotkey_suspended(true)`: 現在のホットキーが**コンボ**ならプラグインを unregister_all(キャプチャ欄にキーが届くように)。`false` で再登録(失敗は eprintln のみ)。特殊トークンはフック側の素通しで対応済み。
- `stop_and_process` の recorder-None 分岐(マイク初期化中に確定された等): silentにidleへ戻さず **`ERR_NO_SPEECH` のエラーフロー**(overlayに2.5秒表示→idle)。

### フロントエンド

- キャプチャ欄: **押下中の修飾キー e.code を Set で追跡**し、bare-tap確定は「そのkeyupまでSetのサイズが1のまま」の場合のみ(両Shift同時押し等の誤検出防止)。`e.key === "AltGraph"` は修飾キーとして扱い AltRight→RAlt にマップ(拒否メッセージを出さない。AltGr配列ではバックエンドが ERR_HOTKEY_REGISTER で拒否し既存ロールバックが働く)。
- overlay: 録音中に settings-changed を受けてもビジュアライザのバー状態をリセットしない(キャプション/モードチップのみ更新)。

# v0.5 追加仕様: リアルタイム字幕 + 認識言語の既定を OS ロケールに

## 設定値

- `live_caption: bool`(既定 `true`): 録音中に途中認識テキストを HUD に表示する。一般タブのスイッチ(`general.liveCaption` / `general.liveCaptionDesc`)。クラウド STT では録音中のリクエストが増える旨を説明文に明記。
- `language` の初回起動時の値は `locale::default_stt_language()`(OS ロケール → whisper コード。zh-CN/zh-TW→`zh`、pt-BR→`pt`、その他は UI コードそのまま)。既存の settings.json は変更しない。
- 認識言語セレクタは「自動判定（非推奨）」(`general.langDetect`, 値 `auto`) + 11 言語(ネイティブ表記、翻訳不要)を main.ts の `STT_LANGS` から動的生成。永続値がリストに無い場合はその値の option を追加して空表示を避ける。`general.langJa` / `general.langEn` は廃止(12言語全ファイルから削除、キーパリティ維持)。

## バックエンド (pipeline.rs / audio.rs)

- `audio::RecorderHandle` は録音バッファ(デバイスレート mono f32)と sample_rate を共有し、`snapshot(from) -> Snapshot { samples(16k i16), end, peak }` で録音を止めずに `from` 以降のコピーを取得できる。
- `start_recording` でハンドル格納後、`settings.live_caption` なら `spawn_live_captions(app, gen)` を起動。ループ:
  1. `CAPTION_PERIOD_MS`(1200ms) 待機 → 録音中でなければ(status≠Recording または generation 不一致)終了。recorder が None(stop_and_process が take 済み)でも終了。
  2. `snapshot(window_start)`。前回から `CAPTION_MIN_NEW_SECS`(0.8s) 未満の新規音声なら skip。`peak < CAPTION_SILENCE_PEAK`(0.01) なら無音として skip(whisper の無音ハルシネーション回避)。
  3. WAV 化して `stt::transcribe`(設定エンジン: local/cloud 共通)。連続 `CAPTION_MAX_FAILURES`(2) 回失敗で当該録音の字幕を打ち切る(サーバ未導入等で再試行し続けない)。
  4. 応答後も録音中なら `caption {text}` を emit。text は `committed + 現在ウィンドウ` (`join_caption`: 境界のどちらかが CJK 文字(かな/漢字/ハングル/全角記号)か空白なら直結、それ以外(ラテン/キリル/ベトナム語等)は空白区切り)。
  5. 開いているウィンドウが `CAPTION_WINDOW_SECS`(20s) を超えたらその text を committed に確定し、`window_start = snapshot.end` で新ウィンドウ開始(長時間口述でもリクエストサイズと遅延を一定に保つ)。
- 最終結果は従来どおり `run_pipeline` が**全録音**を一括認識する(字幕テキストは使わない)。字幕ループは status を一切書き換えない。リクエストは逐次(前の応答が返るまで次を送らない)ので whisper-server への同時要求は最大 1 + 最終認識 1。
- **既知の制約**: 確定時に飛んでいる字幕リクエストは中断できない(HTTP をキャンセルしてもサーバ側の推論は続く)。whisper-server は逐次処理なので、最終認識はその 1 リクエスト(最大 20 秒分の音声)の完了を待つことがある。ウィンドウ上限 20 秒はこの待ちを抑えるためでもある。

## フロントエンド (overlay)

- overlay ウィンドウは 380x190(3行字幕 + 上段 + 波形で約165px、上方向クリップを避ける余裕を持たせる)。カードは**下端に固定**(`bottom: 6px`)し、字幕が無いときは従来の高さ、字幕があると上方向に伸びる。`.viz` は固定高 52px。
- `#caption`: 最大3行(`max-height: calc(1.45em * 3); overflow: hidden`)、更新毎に `scrollTop = scrollHeight` で**末尾(いま話している部分)**を表示。録音中は末尾に点滅カーソル。`:empty` で非表示。
- `caption` イベントは status が recording のときだけ反映。`recording` / `idle` の status で字幕をクリア。transcribing / polishing / done 中は最後の字幕を残す。
- `types.ts`: `Settings.live_caption`, `CaptionPayload { text }`。

# v0.6 追加仕様: ワンクリック自動アップデート

## 方針

- 更新の**確認**は従来どおり `update.rs`(GitHub API `releases/latest`、リリースノート表示用)。更新の**適用**は `tauri-plugin-updater` に任せる(署名検証・ダウンロード・インストーラ起動・再起動を自前実装しない)。
- 署名: minisign 鍵ペア。公開鍵は `tauri.conf.json` の `plugins.updater.pubkey`。秘密鍵は開発者ローカル `~/.tauri/fire-speak.key`(パスワード無し)と GitHub Secrets `TAURI_SIGNING_PRIVATE_KEY`。**秘密鍵を失うと以後の自動更新が不可能になる**(新しい鍵で署名したビルドは既存インストールが拒否する)。
- `bundle.createUpdaterArtifacts: true` で各バンドルに `.sig` を生成。release.yml の tauri-action は `uploadUpdaterJson: true` で `latest.json` をリリースに載せ、4 つのマトリクスジョブがそれぞれ自分のプラットフォームをマージする。`updaterJsonPreferNsis: true`: インストール済みアプリは NSIS 版なので `windows-x86_64` は setup.exe を指す(MSI を重ねると二重インストールになる)。build.yml(手動テストビルド)も署名鍵が必要(無いとバンドルが失敗する)。

## バックエンド (`install_update` コマンド)

1. settings.update の owner/repo から endpoint `https://github.com/{owner}/{repo}/releases/latest/download/latest.json` を組み、`updater_builder().endpoints([...]).on_before_exit(kill_server).build()`。
2. `check()` → None なら `ERR_UPDATE_INSTALL|no update available`。
3. `download_and_install(on_chunk, on_finished)`: `update-progress {phase:"download", downloaded, total, version}` を chunk 毎に、`{phase:"install"}` を完了時に emit。
4. Windows: プラグインが NSIS を `/P`(passive) + `/UPDATE` + 再起動指定で起動し、`std::process::exit(0)` で終了する(RunEvent::Exit は発火しないため whisper-server は `on_before_exit` で停止)。Linux: AppImage は置換、deb/rpm は `dpkg -i` / `rpm` (権限昇格はプラグイン側)、その後 `kill_server` → `app.restart()`。
5. 失敗はすべて `ERR_UPDATE_INSTALL|{detail}`。

## フロントエンド

- ホームのバナーと「アップデート」セクションのボタンは「今すぐアップデート」(`update.install`)。押すと `install_update` を invoke し、`update-progress` を `update.downloading`(`{0}` = `NN%` または `N.N MB`) / `update.installing` として両方の場所に表示。実行中は確認ボタン含め無効化。
- 成功時は UI 更新なし(プロセスが置き換わる)。失敗時は `msg.ERR_UPDATE_INSTALL` をトースト、バナーとセクションに「ダウンロードページを開く」(`update.openDownload`)をフォールバックとして表示。
- ロケール追加(12言語): `update.install` `update.downloading` `update.installing` `msg.ERR_UPDATE_INSTALL`。

## 注意

- 0.5.0 以前のインストールにはこのプラグインが無いので、0.6.0 への更新だけは手動インストール。以後はワンクリック。
- 設定で owner/repo を変えたフォークは、そのリリースが同じ秘密鍵で署名されていなければ更新できない(署名検証で拒否)。

## 設定画面の整理 (v0.6)

- ナビ順: **ホーム / 一般 / 音声認識 / AI整形 / モード / 履歴 / アップデート**(7 項目)。「セットアップ」は廃止し音声認識のローカルパネルへ統合。`SECTION_NAMES` と `nav.*` キーを同期(`nav.setup` 削除)。
- **一般**は 4 カード構成、上から: 言語(表示言語・認識言語) → ホットキー(キー・動作) → 貼り付け(貼り付け方法・クリップボード復元・リアルタイム字幕) → システム(自動起動・履歴保存件数)。見出し h2「一般設定」は廃止(タイトルバーと重複)。カードタイトルのキー: `general.languageHeading` `general.hotkeyHeading` `general.outputHeading` `general.systemHeading`。
- **音声認識(ローカル)**: whisper-server カード(旧セットアップのバッジ/パス/インストールボタン/進捗) → モデル選択カード(1 行 = 選択ラジオ + サイズ/状態 + ダウンロード/再ダウンロードボタン + 進捗バー。未インストール行は選択不可だがダウンロードは可) → 詳細設定。接続テストの行に「設定フォルダを開く」。`stt.modelHint` に一本化(`stt.modelHintPre/Link/Post` 削除)。`msg.ERR_SETUP_REQUIRED` の文言は「音声認識」画面を指すよう 12 言語更新。
- **既定モードに「忍者口調」(id `ninja`, use_llm) を追加**(12 言語、`locale::ninja_mode_for`)。日本語は一人称「拙者」・文末「〜でござる」。既存 settings.json には `modes_version`(新フィールド、既定 0、現行 2)による**一度だけの追補**: `load` 時に `modes_version < 2` かつ id `ninja` が無ければ UI 言語で生成して末尾に追加し、`modes_version = 2` で保存。ユーザーが削除した後に復活しない。
- **ホーム**: アクティブモードはカードグリッドではなく**チップ列**(`.mode-chips` / `.mode-chip-btn`、名前のみ・説明は title)。バナー / ヒーロー / 直近履歴 3 件は維持。

# v0.7 追加仕様: 炎魔法テーマ「Ember Grimoire」+ 新アイコン

## 方針

- 見た目だけの変更。レイアウト・DOM 構造・挙動は v0.6 のまま。`src/theme-ember.css` を `styles.css` の**後**に読み込み、CSS 変数の再定義と主要サーフェスの上書きで実現する(styles.css 自体は最小限の修正: `[hidden] { display: none !important }` の追加)。
- フォント: 見出し `Cinzel`(呪文書・碑文の雰囲気、大文字のみのフォントなので brand / 見出しは大文字表示になる)、本文 `Zen Kaku Gothic New`。Google Fonts から読み込み、オフライン時は Segoe UI / Hiragino Sans にフォールバック(index.html / overlay.html の `<link>`)。
- 配色トークン: 背景 `#0b0708`(黒曜石)、カードは半透明ガラス + 暖色ボーダー、テキストは羊皮紙色 `#f4e9dc`、アクセントは ember `#ff4a00 → #ff7a1f → #ffb347` のグラデーション、金 `#ffd27a` をルーン枠に使用。

## 設定ウィンドウ

- 背景: 左下に残り火の放射グラデーション、`body::before` の feTurbulence ノイズ(opacity 0.16, screen)、`.embers` レイヤー(index.html に 12 個の span、`--x/--s/--d/--delay/--drift` で個別に上昇アニメーション)。
- サイドバー: グリモワールの背表紙。brand は Cinzel + 金→橙のグラデーション文字、🔥 は flicker アニメーション。アクティブなナビ項目は左に発光する ember バー(`::before`)。
- トップバー h1 は Cinzel のグラデーション文字。ステータスピルは録音中に ember グロー。
- カード(`.card` / `.entity-card` / `.history-item`): 半透明 + backdrop-filter、`.card::before/::after` で左上・右下に金のコーナーティック(ゲーム HUD の枠)。`.card-title` は Cinzel 大文字・字間 1.6px。`.section-heading` は右にフェードするラインを持つ。
- ボタン: `.btn-primary` は ember グラデーション + 内側の金リング + hover で光沢スイープ(`::after`、`overflow: hidden` で内側にクリップ)。
- ホームのヒーロー: 左奥に**逆回転する 2 本のルーンリング**(`::before` 点線 28s、`::after` conic-gradient 12s reverse)。ホットキーのチップは Cinzel のルーンプレート。モードチップのアクティブは ember グラデーション。
- select は `background-color` で上書きする(shorthand `background` を使うと矢印画像の no-repeat / position が失われタイル表示になる)。
- `prefers-reduced-motion: reduce` で火の粉・炎・リングのアニメーションを停止。

## 録音 HUD (overlay.css 末尾に追記)

- カードは黒曜石 + 左下の ember グロー、録音中は ember のボーダーと外側/内側グロー。
- 録音中は rec-dot の周囲に点線のルーンリング(`::before`, 6s 回転)。モードチップは Cinzel の金文字(呪文名)。波形バーは金→橙→朱のグラデーション + グロー、done は緑。

## アイコン

- `src-tauri/icons/source.svg`(1024px)から `npm run tauri icon -- src-tauri/icons/source.svg` で全サイズ生成。
- v0.8.0 で作り直し: **黒い円盤は無し**(16〜32px で「黒丸に炎」に見えるため)。シルエットは**太い炎のリング**(stroke 64px + グロー、両縁に金の細線、8 本の暗いルーン切り欠き、8 個の金の点)そのもの。内側に太い金→橙の六芒星(stroke 20px + ソフトグロー)、リング上部から立ち上がる炎(朱→橙→金→白、白熱コア、火の粉)。リング内側だけ暗いヘイズで炎のコントラストを確保し、外縁に向かって透明にフェード。**魔法陣がアイコン**: リング(stroke 80)は枠いっぱい(外縁 r=504)、六芒星は内縁に接する大きさ(外接半径 400、stroke 26)、内側に点線の小円、炎は中央に 66% スケールで小さめ。確認は 256/128/64/48/32/24/16px のシートで行う。

## AI整形の「キー未設定」を見える化 + Ollama プリセット (v0.7)

- 問題: `run_pipeline` はアクティブプロバイダに API キーが無いと**何も表示せず**文字起こしをそのまま貼り付けていた(履歴では raw_text == final_text)。ユーザーは整形が動いていると思い込める。
- 修正: `llm::is_configured(provider)` = API キーあり、または **kind≠anthropic かつ base_url が localhost/127.0.0.1**(Ollama 等はキー不要。Bearer ヘッダはキーがある時だけ付ける)。`use_llm` のモードでプロバイダが無い/未設定なら `WARN_LLM_NO_KEY` を done メッセージに載せる(HUD: 「✓ 貼り付けました — AI整形はスキップしました（API キー未設定）」)。
- エラーキー: `ERR_LLM_NO_KEY|{provider name}`(接続テスト時。旧 `ERR_INTERNAL|LLM API key not set`)、`ERR_STT_NO_KEY`(クラウド STT)。12 言語に `msg.ERR_LLM_NO_KEY` `msg.ERR_STT_NO_KEY` `msg.WARN_LLM_NO_KEY` 追加。
- 既定プロバイダに **Ollama (local, free)**(id `ollama`, kind openai, `http://localhost:11434/v1`, model `qwen2.5:7b`, キー空)を追加。既存 settings.json には defaults version 3(`modes_version` フィールドを流用、v0.6 の 2 = 忍者モード)で一度だけ追補(id `ollama` が無い場合のみ)。

# v0.8 追加仕様: GPU(CUDA)版 whisper-server の自動選択

## 背景

- 実機(RTX 4070 Ti SUPER, 16 論理 CPU)で「変換が遅い」「字幕が出ない」: whisper large-v3-turbo を **CPU 4 スレッド**で回していたため、字幕 1 リクエストに数秒かかり短い発話では最初の字幕が出る前に離していた。whisper.cpp のリリースには CUDA ランタイム DLL 同梱の Windows x64 CUDA ビルドがあり、NVIDIA ドライバだけで動く。
- NPU / DirectML について: whisper.cpp の配布バイナリに NPU 対応は無い(OpenVINO はエンコーダのみで別ランタイムが必要)。miomail の方式(ONNX Runtime + EP デバイス列挙: OpenVINO/VitisAI/QNN/DirectML)を fire-speak に持ち込むには STT エンジンを ONNX Runtime 系(sherpa-onnx 等)に差し替える必要があり、別バージョンの作業とする。v0.8 は whisper.cpp の CUDA ビルドまで。

## 設定 / 状態

- `stt.local.gpu: bool`(既定 true): 詳細設定のスイッチ「GPU を使う（NVIDIA CUDA）」。
- `SetupStatus.gpu_available`(Windows: `%SystemRoot%\System32\nvcuda.dll` の存在 = NVIDIA ドライバあり。他 OS は false)、`SetupStatus.server_backend`("cuda" = サーバ exe の隣に `ggml-cuda.dll` がある / "cpu" / "")。
- `default_threads()` = 論理 CPU 数 / 2 を 4..=8 に丸める(新規インストールのみ。既存の 4 は変更しない)。

## サーバのダウンロード (setup.rs)

- `releases/latest` 単発ではなく **`releases?per_page=10` を新しい順に走査**し、draft を除き、最初に条件を満たすアセットを持つリリースを使う(v1.9.4 のようにアセット無しの latest や、CUDA ビルドの無いリリースをスキップ)。
- `want_cuda = settings.stt.local.gpu && gpu_available()` のとき `pick_server_asset(assets, true)`: 名前に `cublas` または `cuda`、`x64`、`arm64` を含まず `.zip`。`12.` を含むものを優先(`whisper-cublas-12.4.0-bin-x64.zip` 約 670 MB、新命名 `whisper-bin-win-cuda-12.x-x64.zip` も一致)。どのリリースにも無ければ CPU ビルドにフォールバック。
- **ステージング方式**: `bin.new/` にダウンロード・展開し、サーバ exe が見つかることを確認してから、稼働中サーバを kill(spawn_blocking)→ 旧 `bin/` 削除 → `bin.new` を `bin` にリネーム。途中で失敗したら `bin.new` を消して旧サーバはそのまま残す。CPU/CUDA の DLL 混在も起きない。
- 起動時: `gpu=false` かつ `server_backend == "cuda"` のときだけ `--no-gpu` を付ける(ユーザー指定の古い server_path が未知フラグで落ちないように)。`-fa`(flash attention)は whisper-server の既定で有効。シグネチャに `gpu=` を含め、切替時にサーバを再起動する。

## フロントエンド

- サーバカード: インストール状態の隣に **ビルドのバッジ**(`setup.backendCpu` / `setup.backendCuda`)。`gpu_available && gpu 設定 on && backend≠cuda` のとき、ヒント `setup.gpuHint` と主ボタン **「GPU 版をインストール」**(`setup.installGpu`、同じ download_whisper_server コマンド。バックエンドが CUDA アセットを選ぶ)を表示。それ以外は従来の インストール / 再インストール。
- ロケール追加(12 言語): `setup.backendCpu` `setup.backendCuda` `setup.gpuHint` `setup.installGpu` `stt.useGpu` `stt.useGpuDesc`。
- 字幕ループの待機 `CAPTION_PERIOD_MS` を 1200 → 700ms(GPU では 1 リクエスト 1 秒未満なので更新が滑らかになる。CPU では逐次実行なので自然に間引かれる)。


# v0.8.1 追加仕様: whisper-server の事前起動・CPU 版の自動 GPU 化・クラウドプリセット

## 背景

- v0.8.0 を入れた実機でも「認識が遅い」「字幕が出ない」: v0.8 以前に入れた **CPU 版 whisper-server が残ったまま**(CUDA 版は新規ダウンロード時しか選ばれない)で、さらにサーバは初回の文字起こし時に起動するため、アプリ起動後最初の発話は large-v3-turbo(1.6GB)の読み込みを待っていた。

## バックエンド (setup.rs / lib.rs)

- `setup::prewarm(app)`: engine=`local` かつサーバ・モデルが解決できるとき、バックグラウンドで `ensure_server` を呼ぶ。失敗はログのみ(次の文字起こしで同じエラーが通常経路で出る)。
- `setup::on_startup(app)`(`setup()` の最後): `prewarm` した上で、次をすべて満たすとき CUDA 版を**自動でダウンロードして入れ替え**、完了後に再度 `prewarm`:
  engine=`local`、`stt.local.gpu`=true、`stt.local.server_path` が空(アプリ管理のサーバ)、`gpu_available()`、インストール済みサーバの `server_backend`=`cpu`。
  ステージング方式(v0.8)なので、ダウンロード中は CPU 版がそのまま使える。進捗・完了/失敗は通常の `download-progress` で通知される(設定画面を開いていればトースト)。
- `download_whisper_server` は同時に 1 本だけ: 実行中に呼ばれた 2 本目は即 `Ok(())` を返し、進行中のダウンロードに合流する(フロントは `download-progress` の done/error で後処理する)。
- コマンド `download_whisper_server` / `download_model` は成功後に `prewarm`(サーバは実際にダウンロードした呼び出しだけ。合流した呼び出しはしない)。
- **入れ替え中は起動しない**: サーバの kill→`bin/` 削除→rename の間は `SERVER_SWAPPING` を立てる。`ensure_server_blocking` は `server_starting` を取った後に `SERVER_SWAPPING` を確認し、立っていれば手放して待つ。入れ替え側はフラグを立てた後、進行中の起動(`server_starting`)が終わるのを待ってから kill する(双方が自分のフラグを書いてから相手のフラグを読む、SeqCst)。古い exe を掴んだまま起動されて Windows で `bin/` が中途半端に消える事故を防ぐ。どちらのフラグも Drop ガードで確実に下ろす。
- `apply_settings` で `stt` が変わったとき: engine=`local` なら `prewarm`(`ensure_server` の起動シグネチャ比較により、起動に関わる値が変わったときだけ再起動)、`cloud` ならサーバを停止してメモリ/VRAM を解放。
- トレードオフ: ローカルエンジンではアプリ起動中ずっとモデルが常駐する(large-v3-turbo で RAM または VRAM 約 2GB)。

## フロントエンド

- クラウド STT パネルの先頭に**プリセット**行: `OpenAI`(`https://api.openai.com/v1` / `whisper-1`)、`Groq`(`https://api.groq.com/openai/v1` / `whisper-large-v3-turbo`)。Base URL とモデルだけを埋め、API キーは変更しない。
- ロケール追加(12 言語): `stt.preset` `stt.presetHint`。

# v0.9 追加仕様: AMD GPU (Vulkan) / AMD NPU 対応

## 背景

- Ryzen AI Max+ 395(Strix Halo: Radeon 8060S iGPU 40CU + XDNA2 NPU 50TOPS)で「遅い」: v0.8 までの GPU 対応は NVIDIA CUDA のみで、AMD 機は CPU 版だった。
- whisper.cpp 公式リリースに Windows の Vulkan / NPU ビルドは無い。AMD のローカル AI サーバ **Lemonade** が whisper.cpp のフォーク `lemonade-sdk/whisper.cpp-rocm` で `whisper-server` の Vulkan / NPU / ROCm ビルドを配布しており(HTTP API は本家と同じ)、NPU 用のコンパイル済みエンコーダ(`.rai`)を AMD が Hugging Face `amd/whisper-*-onnx-npu` で公開している(Lemonade の `server_models.json` の `npu_cache` と同じ)。これをそのまま使う。
- 実測(RTX 4070 Ti SUPER, large-v3-turbo, 11 秒の日本語): Vulkan 0.15〜0.2 s / CUDA 0.16 s / CPU 8 スレッド 6.5 s。Vulkan の初回リクエストはシェーダのコンパイルで約 3.5 s → 事前起動時にウォームアップする。
- NPU ビルドはエンコーダだけを NPU で動かし、デコーダは CPU。8060S の iGPU(Vulkan)はモデル全体を GPU で動かすので、速度は Vulkan の方が有利と見込み、**自動選択では NPU を選ばない**(省電力を求めるユーザーが明示的に選ぶ)。AMD 実機での速度比較は未実施。

## 設定

- `stt.local.accel: "auto" | "cuda" | "vulkan" | "npu" | "cpu"`(既定 `"auto"`)。v0.8 の `stt.local.gpu` は残し、`auto` かつ `gpu=false` なら CPU(既存ユーザーの「GPU オフ」を尊重)。UI で選ぶと `gpu = accel !== "cpu"` に揃える。
- 解決(`hw::resolve_accel`): 明示値はそのプラットフォームで選べるもの(`hw::supported_accels`: Windows x64 = auto/cuda/vulkan/npu/cpu、Linux x64 = auto/vulkan/cpu、その他 = auto/cpu)ならそのまま。`auto` = NVIDIA ドライバ → `cuda`、**型番付きの AMD Radeon**(8060S / 780M / RX … 名前に数字を含む) + Vulkan ローダ → `vulkan`、それ以外 → `cpu`。デスクトップ Ryzen 内蔵の無印「AMD Radeon(TM) Graphics」(2CU)は CPU より遅いことがあるので対象外。Linux は sysfs で iGPU の規模を判別できないため auto では Vulkan を選ばない(明示選択のみ、実機未検証)。

## ハードウェア検出 (hw.rs)

- Windows: `System32\nvcuda.dll`(NVIDIA)、`System32\vulkan-1.dll`(Vulkan ローダ)、PowerShell(CREATE_NO_WINDOW、8 秒でタイムアウトして kill)で `Win32_VideoController` の名前一覧と `Win32_PnPEntity` の `NPU Compute Accelerator` デバイス数(Lemonade と同じ判定 = ドライバ導入済み NPU)。プロセス内で 1 回だけ実行してキャッシュ。起動時の `sync_server_build` が blocking スレッドで先に叩くので設定画面は通常待たない。
- `SetupStatus` に `hw`(nvidia/amd_gpu/vulkan/npu/gpus)、`accel_options`、`effective_accel`、`npu_cache_ready` を追加。`server_backend` は `npu`(flexmlrt.dll)> `cuda`(ggml-cuda)> `vulkan`(ggml-vulkan)> `cpu` の順で判定。

## サーバのビルド

- ダウンロード元: `cuda`/`cpu` は従来どおり ggml-org/whisper.cpp の最新リリース。`vulkan`/`npu` は **`lemonade-sdk/whisper.cpp-rocm` の `v1.8.4` に固定**(`whisper-v1.8.4-windows-vulkan-x64.zip` 20MB、`whisper-v1.8.4-windows-npu-x64.zip` 3MB、Linux は `whisper-v1.8.4-linux-vulkan-x86_64.tar.gz`)し、GitHub が公開するアセットの SHA-256 と照合(不一致は `ERR_DOWNLOAD_FAILED`)。展開・ステージング・入れ替えは v0.8/v0.8.1 と同じ。
- 再利用: `hw::build_satisfies(installed, wanted)` = 同じビルド、または CPU を CUDA/Vulkan ビルドで `--no-gpu` 実行。NPU ビルドは CPU 用に流用しない。
- 起動引数: CPU が欲しいのに CUDA/Vulkan ビルドなら `--no-gpu`。起動シグネチャに `backend=` と `no_gpu=` を含める(入れ替え直後の旧ビルド起動を次回呼び出しで再起動させる)。
- ヘルスチェックの上限を 20 → **60 秒**(NPU ビルドは 700MB のエンコーダを読む。子プロセスが死んだ場合は従来どおり即エラー)。

## NPU エンコーダ (.rai)

- 置き場所はモデルと同じフォルダの `ggml-{name}-encoder-vitisai.rai`(NPU ビルドの whisper-server がこの名前で探す)。取得元: tiny/base/small/medium = `amd/whisper-{name}-onnx-npu`、large-v3-turbo = `amd/whisper-large-turbo-onnx-npu`(約 708MB)。`.rai.part` に書いてから rename。
- NPU ビルドでエンコーダが無いときは起動せず `ERR_NPU_CACHE_MISSING`。管理外モデル(パス上書き)や対応表に無いモデルは `ERR_NPU_NO_CACHE`。
- コマンド `download_npu_cache`(進捗 `download-progress` の kind=`npu`, name=`npu-encoder`、同時 1 本・合流)。サーバのビルドと同時に走りうるので進捗バーは別(`#npu-progress`)。
- `setup_status` は async + `spawn_blocking`(初回はハードウェア検出で最大 8 秒かかりうるため、メインスレッドで実行しない)。

## 自動同期 (`setup::sync_server_build`)

- 起動時、`stt` 設定変更時(engine=local)、モデルのダウンロード後に実行。hw を blocking スレッドで確定 → engine=local のときだけ:
  1. アプリ管理サーバ(server_path 空)がインストール済みで `build_satisfies` を満たさなければ、欲しいビルドをバックグラウンドでダウンロード・入れ替え(未インストールからの初回導入は従来どおりユーザー操作)。実行中のダウンロードに合流した場合はその終了を待ち、改めて `build_satisfies` を確認して満たさなければ終了(後続の同期に任せる)。
  - 入れ替えが済むまでは旧ビルドがそのまま応答する(旧ビルドの性質で動く。例: NPU→GPU に切り替えた直後はまだ NPU ビルド)。
  2. NPU で、アクティブモデルのエンコーダが無ければダウンロード。
  3. `prewarm`。
- `prewarm` はサーバ起動後、CPU 以外なら 1 秒の無音で 1 回推論してカーネルをコンパイルさせる(結果は捨てる)。

## フロントエンド

- サーバカード: バッジに `setup.backendVulkan` / `setup.backendNpu` を追加。**処理デバイス**セレクタ(`stt.accel`、選択肢 `stt.accelAuto/Cuda/Vulkan/Npu/Cpu`、`accel_options` のものだけ)と検出結果の行(`stt.accelDetected` + `stt.noGpu` / `stt.npuFound` → 解決先の短い名前 CUDA/Vulkan/NPU/CPU)。変更は即保存して setup_status を取り直す。
- インストール済みビルドが合わないとき `setup.buildHint` と主ボタン `setup.installBuild`(短い名前を埋め込む)。NPU でエンコーダが無いとき `setup.npuCacheHint` + ボタン `setup.npuCacheInstall`。進捗はサーバカードの進捗バーを共用し、完了/失敗は `setup.npuCacheDownloaded` / `setup.npuCacheFailed`。
- 詳細設定の「GPU を使う」スイッチは廃止(`stt.useGpu` `stt.useGpuDesc` `setup.gpuHint` `setup.installGpu` を 12 言語から削除)。`msg.ERR_NPU_CACHE_MISSING` `msg.ERR_NPU_NO_CACHE` を追加。

## 未検証

- AMD 実機(Vulkan on Radeon / NPU)での動作と速度。開発機(RTX 4070 Ti SUPER)では Lemonade の Vulkan ビルドが NVIDIA 上で動くこと、ハッシュ・URL、UI(スタブで Strix Halo 構成を再現)を確認。
- NPU には AMD の NPU ドライバ(Ryzen AI 対応ドライバ)が必要。未導入だと検出行に「NPU あり」が出ず、選んでもサーバが起動しない(`ERR_SERVER_DIED`)。

# v0.9.1 追加仕様: 初回セットアップカード(ホーム)

## 背景

- インストール直後にウィンドウを開いても何をすればいいか分からない: ホームは「音声入力の準備ができています」と表示するのに、実際は「音声認識」画面で whisper-server とモデルを別々にインストールしないと動かなかった。

## バックエンド

- `setup::recommended_model(accel)`: GPU/NPU(`cuda`/`vulkan`/`npu`)は `large-v3-turbo`、CPU は `small`(turbo は CPU だと 11 秒の音声に約 6 秒)。`SetupStatus.recommended_model` で返す。
- コマンド `quick_setup`: ハードウェア検出(blocking スレッド)→ 必要なら (1) この PC の処理デバイスに合うサーバビルド(未インストール、または `build_satisfies` を満たさない場合。server_path 上書き時は存在すれば可) → (2) モデルが 1 つも無ければ推奨モデル → (3) NPU ならエンコーダ、の順に実行して `prewarm`。済んでいる手順は飛ばす。進捗は既存の `download-progress`(kind server / model / npu)。実行中の 2 回目の呼び出しは即 `Ok`。実行中のダウンロードに合流した場合はその終了を待ち、サーバは `build_satisfies`・モデルは存在を再確認して、満たさなければ `ERR_SETUP_REQUIRED`。失敗時はそのダウンロードのエラーを返す(`download-progress` の error でトースト済みなので、フロントは `ERR_SETUP_REQUIRED` のときだけ `onboard.failed` を出す)。
- `download_model` も同じモデルの同時ダウンロードを合流させる(同じ `.part` への同時書き込み防止。`MODEL_DOWNLOADS`)。

## フロントエンド

- ホーム先頭に `#home-setup` カード。engine=`local` で手順のどれかが未完了のあいだ、ヒーロー(`#home-hero`)の代わりに表示する。
  - タイトル `onboard.title`、説明 `onboard.desc`、検出行 `onboard.detected`(GPU 名 → 短いビルド名)。
  - 手順リスト: `onboard.stepServer`(ビルド名)/ `onboard.stepModel`(推奨モデル名)/ NPU 時 `onboard.stepNpu`。各行に ✓/○、サイズ(サーバの概算: cuda 643MB, vulkan 20MB, npu 3MB, cpu 8MB。NPU エンコーダは large-v3-turbo のみ 708MB)、実行中は進捗バー(`activeDownloads` と同じキー)。
  - 主ボタン `onboard.start`(未完了手順の合計サイズ)→ 実行中 `onboard.running`。完了で `onboard.done`(ホットキー名入り)をトーストし、カードが消えてヒーローに戻る。失敗は `onboard.failed`(ダウンロード系エラーは各イベントでトースト済みなので重複させない)。
  - 副ボタン `onboard.useCloud`: engine を cloud にして保存し「音声認識」画面(クラウド設定)へ移動。
- `msg.ERR_SETUP_REQUIRED` を「ホーム画面の『セットアップを開始』を押してください」に変更(12 言語)。
