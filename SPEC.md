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

# v0.9.2 追加仕様: ホームの「動作状況」カード

- 背景: ホームを見ても何がインストール済みで何が未設定か分からない。
- ホームのヒーローの下に `#home-status`(見出し `sys.heading`)。初回セットアップカードが出ている間は隠す(同じ情報をそちらが出す)。各行 = 状態マーク(✓ ok / ! warn / ✕ missing)+ 項目名 + 値 + 「変更」ボタン(該当画面へ移動)。
  - ローカル: 音声認識エンジン(`sys.build` のビルド名 + `sys.running` 起動中 / `sys.idle` 停止中、未導入は ✕)、音声モデル(モデル名、未導入は ✕)、処理デバイス(GPU 名 → 解決先。入っているビルドが合わなければ !)、NPU 時は NPU エンコーダ。
  - クラウド: `sys.cloud`(ホスト / モデル、API キーが空なら ✕)。
  - 認識言語: 言語名、`auto` は ! `sys.langAuto`(遅い)。
  - AI整形: アクティブモードが LLM を使わないなら `sys.llmOff`、使えるプロバイダ(`llm::is_configured` と同じ判定をフロントで再現)ならプロバイダ名(モデル)、使えなければ ! `sys.llmNoKey`。
- `SetupStatus.server_running`(管理中の whisper-server が生存)。バックエンドはサーバ起動成功時と `kill_server` で停止したときに `server-state`(bool)を emit し、フロントは setup_status を取り直す。ホームに切り替えたときも取り直す。
- ロケール追加(12 言語): `sys.*` 18 キー。

# v0.9.3 追加仕様: AI整形プロバイダのプリセット / Poolside / 畳んだカード

- 背景: Poolside の API(`https://inference.poolside.ai/v1`、OpenAI 互換 chat/completions、モデル `poolside/laguna-s-2.1`)を直接使いたい。プロバイダを手で追加すると URL とモデル名を打ち込む必要があった。また各プロバイダカードが常に全項目を開いていて縦に長く、並ぶと見づらい。
- 「AI整形」セクションの見出し直下に**プリセット**行(`llm.preset`、クラウド STT と同じ `.preset-row`)。並び順は **Poolside** / Claude / OpenAI / OpenRouter / Groq / Ollama。押すと名前・種別・Base URL・モデルを埋めた**新しいプロバイダを追加**し(API キーは空)、アクティブが無ければそれをアクティブにし、そのカードを開いた状態で末尾に追加してスクロール。「＋ プロバイダを追加」も同じ `addProvider` を通る。ヒント `llm.presetHint`。
  - 値: Poolside = `https://inference.poolside.ai/v1` / `poolside/laguna-s-2.1`、Claude = anthropic / `https://api.anthropic.com` / `claude-haiku-4-5`、OpenAI = `https://api.openai.com/v1` / `gpt-4o-mini`、OpenRouter = `https://openrouter.ai/api/v1` / `poolside/laguna-s-2.1:free`、Groq = `https://api.groq.com/openai/v1` / `llama-3.3-70b-versatile`、Ollama = `settings::ollama_provider` と同じ。
- **プロバイダカードは既定で畳む**(`.entity-card.is-collapsed`、メモリ上の `expandedProviders` 集合に入っている id だけ開く。再起動で全部畳む)。畳んだ状態は 1 行 = アクティブのラジオ + 名前(編集可) + `entity-summary`(モデル名、title に Base URL。`providerReady` でなければバッジ `llm.noKey`)+ 開閉ボタン(▸/▾、title `llm.edit`)+ 削除。開くと種別/モデル/URL/API キーのグリッドとテスト行が出る。グリッドは `form-grid-inline`(1 列、各行はラベル 96px + 入力欄の横並び)で縦を詰める。
- 初回起動の既定プロバイダ(`default_providers`)の**先頭**に `poolside`(Laguna S 2.1 (Poolside))を追加(Poolside / Claude / OpenRouter / Ollama の 4 件)。既存ユーザーの設定は変更しない(プリセットから追加できる)。
- ロケール追加(12 言語): `llm.preset` `llm.presetHint` `llm.edit` `llm.noKey`。
- README: 先頭に英語の紹介文、初回セットアップ手順をホームのセットアップカード前提に更新、Poolside のキー発行先を追記。

# v0.9.4 追加仕様: モードの有効/無効と組み込み「なし」、HTML モーダル、アップデートのモーダル表示

## モード

- `Mode.enabled: bool`(serde default true)。モードカードの「AI整形を使う」トグルを**「有効」トグル**に置き換える(`modes.enabled`)。すべてのモードは LLM を使う(`use_llm` はフィールドとして残すが UI から消え、常に true)。
- **組み込みの「なし」モード**(`settings::NONE_MODE_ID` = `"none"`、`modes` には保存しない)。`active_mode_id` が `"none"`、存在しない id、または `enabled=false` のモードを指すとき、パイプラインは LLM を使わず文字起こしをそのまま貼り付ける(`pipeline` の fallback)。
  - ホームのチップ列とモード管理の先頭に「なし」(`modes.none`)を固定表示。モード管理の行は編集・無効化・削除できず、クリックで選択(`modes.activateHint`)。
  - トレイメニューの先頭に「なし」の CheckMenuItem(`locale::tray_labels` の 3 つ目)。
- 無効にしたモードは**グレーアウト**: モードカード `.entity-card.is-disabled`、ホームのチップ `.mode-chip-btn.is-disabled`(押せない、title `modes.disabledHint`)、トレイ項目は enabled=false。アクティブなモードを無効にすると `active_mode_id = "none"`。削除時も同じ(以前は先頭のモードへ)。「最後のモードは削除できません」の制限と `modes.lastModeError` は廃止。
- `set_active_mode` は `"none"` または enabled なモードだけ受け付ける(他は `ERR_INTERNAL|mode not found`)。
- 既定モードから `raw`(そのまま)を外す(6 件: polish / to_en / to_ja / terminal / business / ninja)。
- 移行(`modes_version` 4): `use_llm=false` かつ指示が空のモード(既定の `raw`)を削除、残りは `use_llm=true` に揃える。`active_mode_id` が存在しないモードを指していたら `"none"`。
- ホームの動作状況カード: アクティブが「なし」/無効/削除済みなら `sys.llmOff` にモード名「なし」を入れる。
- ロケール: 追加 `modes.enabled` `modes.none` `modes.disabledHint` `modes.activateHint`、`modes.hint` を更新、削除 `modes.useLlm` `modes.lastModeError`。

## HTML モーダル(JS ダイアログ全廃)

- `window.confirm` / `alert` / `prompt` は使わない。`<dialog class="modal" id="confirm-dialog">`(`<form method="dialog">`、メッセージ + キャンセル(`common.cancel`)+ 主ボタン)を `confirmDialog(message, okLabel = common.ok, danger = false): Promise<boolean>` で開く。Esc / 背景クリックは false。
- 置き換え箇所: プロバイダ削除(主ボタン `common.delete`、danger)、モード削除(同)、履歴の全削除(同)、終了(`sidebar.quit`、danger)。
- ロケール追加: `common.ok` `common.cancel`。

## アップデートのモーダル表示

- 起動時の自動チェック(`autoCheckUpdate`)で新版を検出したら、従来のホームのバナーに加えて `<dialog id="update-dialog">` を**モーダルで表示**(`update.modalTitle`、`update.available`、リリースノートを `textContent` で、ボタン「後で」`update.later` / 「今すぐアップデート」`update.install`)。「今すぐアップデート」は `startInstallUpdate()`(進捗はバナーとアップデート画面に出る)。同じバージョンについてはセッション中 1 回だけ。手動チェックではモーダルを出さない(結果はアップデート画面に出る)。
- ロケール追加: `update.modalTitle` `update.later`。

# v0.9.5 追加仕様: 設定保存の堅牢化と可視化

- 背景: あるマシンで `settings.json` が 9/8 から一度も更新されず(API キーを含む全設定が再起動のたびに失われる)、UI には 2.6 秒のトーストしか出ていなかった。原因は再現できていないため、保存を堅牢にし、失敗を見える化してログに残す。
- `settings::save`: `settings.json.tmp` に書いて `rename` で置き換える(アトミック)。失敗(書き込み / rename / 読み戻し検証の不一致)は 120ms×試行回数の待ちで最大 5 回まで再試行。最終的に失敗したら `ERR_SAVE_SETTINGS|{detail}` を返し、`LAST_SAVE_ERROR` に記録してログに書く。成功で `LAST_SAVE_ERROR` をクリア。
- `settings::log_line(app, msg)`: `{app_config_dir}/fire-speak.log` に `YYYY-MM-DD HH:MM:SSZ vX.Y.Z msg` を追記(256KB を超えたら作り直し)。起動時に解決した設定ファイルのパス、初回作成 / 移行時の保存失敗、パース失敗による既定値再作成を記録する。転写テキストやキーは書かない。
- `SetupStatus.last_save_error: String`(空 = 直近の保存成功)。
- ホームの動作状況カードの先頭に、`last_save_error` が空でないとき ✕ 行 `sys.save` / `sys.saveFailed`(エラー文言入り、「変更」は音声認識画面 = 設定フォルダを開くボタンがある)。フロントは保存失敗のトースト後に `refreshSetupStatus()` を呼んで行を出す。
- エラートーストの表示時間を 6 秒に延長(通常は 2.6 秒のまま)。
- ロケール追加(12 言語): `sys.save` `sys.saveFailed`。

# v0.10.0 追加仕様: Intel NPU (OpenVINO) 対応 — 実機未検証

## 背景

- 処理デバイスの一覧に Intel NPU が無かった(Windows x64 は CUDA / Vulkan / AMD NPU / CPU の固定)。Core Ultra の NPU(Intel AI Boost)で動かしたい。
- whisper.cpp は OpenVINO バックエンド(`WHISPER_OPENVINO=1`)でエンコーダを OpenVINO デバイスで実行できる(公式の対応表記は CPU / Intel GPU、NPU は OpenVINO の Whisper パイプラインでは実績ありだが whisper-server では未確認)。公式リリースに OpenVINO ビルドは無いので**自前で CI ビルド**する。
- Intel NPU 搭載機が手元に無いため、本仕様は**ビルド・配布・検出・UI・起動引数まで**を実装し、NPU 上での実行は Intel 機が用意でき次第確認する。

## CI (`.github/workflows/whisper-openvino.yml`, 手動起動)

- `server` ジョブ(windows-latest): `ggml-org/whisper.cpp` を入力タグ(既定 `v1.9.4`)で checkout、OpenVINO Runtime の Windows アーカイブ(既定 2026.4.0、`.sha256` で検証)を展開し、`cmake -DWHISPER_OPENVINO=1 -DOpenVINO_DIR=<ov>/runtime/cmake -DBUILD_SHARED_LIBS=OFF -DGGML_NATIVE=OFF` で `whisper-server` をビルド。`Release/` に `whisper-server.exe` + OpenVINO の全ランタイム DLL(`runtime/bin/intel64/Release/*` = コア + CPU/GPU/NPU/AUTO プラグイン)+ TBB DLL を入れて `whisper-<tag>-windows-openvino-x64.zip` にする。
- `encoders` ジョブ(ubuntu-latest、モデル行列 tiny/base/small/medium/large-v3-turbo): `models/convert-whisper-to-openvino.py` で IR を生成し、`ov.save_model(compress_to_fp16=True)` で FP16 に圧縮して `ggml-<model>-encoder-openvino.zip`(xml + bin)にする。
- `release` ジョブ: このリポジトリに **prerelease** `whisper-openvino-<whisper_tag>-ov<ov_version>`(既定 `whisper-openvino-v1.9.4-ov2026.4`)を作成/更新し、全アセットと `SHA256SUMS.txt` を載せる。prerelease なのでアプリのアップデート確認(`releases/latest`)には出ない。

## 検出 (hw.rs)

- `Hardware.intel_npu`: `Win32_PnPEntity` で `Name LIKE '%AI Boost%'`(Intel NPU ドライバのデバイス名 "Intel(R) AI Boost")。既存の PowerShell プローブに追加。
- `supported_accels`(Windows x64)に `openvino` を追加: `auto / cuda / vulkan / npu / openvino / cpu`。`resolve_accel` は明示選択のみ(auto では選ばない。AMD NPU と同じ)。
- `build_satisfies`: OpenVINO ビルドは `--ov-e-device` 無しなら通常の CPU ビルドなので、`wanted=cpu` を満たす(CUDA/Vulkan と同様。AMD NPU ビルドは従来どおり流用しない)。

## サーバ (setup.rs)

- `server_backend`: exe の隣に `openvino*.dll` があれば `openvino`(flexmlrt / ggml-cuda / ggml-vulkan より先に判定)。
- ダウンロード元: `accel=openvino` は `firemio/fire-speak` の release `OPENVINO_RELEASE_TAG` のアセット `OPENVINO_SERVER_ASSET`、SHA-256 を `OPENVINO_SERVER_SHA256` と照合(Lemonade と同じ経路 `install_server_archive`)。
- 起動: `backend=openvino && accel=openvino` のとき `--ov-e-device NPU` を付け、モデルの隣に `ggml-{name}-encoder-openvino.xml` が無ければ `ERR_NPU_CACHE_MISSING`。ヘルスチェックの上限はこのときだけ **600 秒**(初回は OpenVINO が NPU 用にエンコーダをコンパイルする。whisper.cpp がモデルの隣の `…-encoder-openvino-cache` にキャッシュするので 2 回目以降は速い)。起動シグネチャに `ov=` を含める。
- エンコーダ: `encoder_cache_path(model, accel)` = AMD `…-encoder-vitisai.rai` / Intel `…-encoder-openvino.xml`。`npu_cache_ready` と `download_npu_cache`(進捗 kind `npu`)は両方を扱う。Intel は `ggml-{model}-encoder-openvino.zip` を `.part` にダウンロード → SHA-256 照合(`openvino_encoder_asset` の表)→ モデルのフォルダに展開 → zip 削除。管理外モデル・表に無いモデルは `ERR_NPU_NO_CACHE`。
- `sync_server_build` / `quick_setup` の「NPU ならエンコーダも取得」は `needs_encoder_cache(accel)`(npu | openvino)。`recommended_model` は openvino も GPU/NPU 扱い(large-v3-turbo)。

## フロントエンド

- `stt.accelOpenvino`(処理デバイスの選択肢)、`setup.backendOpenvino`(ビルドのバッジ)、`stt.intelNpuFound`(検出行)。`ACCEL_SHORT.openvino = "Intel NPU"`。
- 「NPU エンコーダ」の行・初回セットアップの手順・動作状況カードの判定は `needsEncoderCache(accel)`。サイズ(実アセット): サーバ 81 MB、エンコーダ zip(FP16) tiny 14 / base 36 / small 155 / medium 541 / large-v3-turbo 1110 MB。
- `setup.npuCacheHint` を AMD 固有の文言から汎用(0.7〜1.3 GB)に変更(12 言語)。

## 検証済み(2026-10-01、開発機 = NVIDIA/AMD 無し Intel NPU 無し)

- CI が出した `whisper-v1.9.4-windows-openvino-x64.zip`(81 MB、`whisper-server.exe` + openvino.dll + CPU/GPU/NPU/AUTO プラグイン + NPU コンパイラ + TBB)と `ggml-tiny-encoder-openvino.zip` を手で展開し、`whisper-server -m ggml-tiny.bin --ov-e-device CPU` で起動 → ログに `whisper_openvino_init: … device = CPU, cache_dir = …-encoder-openvino-cache` / `OPENVINO = 1`、`/inference` が応答。ビルド・パッケージ・エンコーダ IR・起動引数は正しい。

## 未検証・既知の懸念

- Intel NPU 上での実行そのもの(OpenVINO NPU プラグインが whisper エンコーダ IR をコンパイルできるか、large-v3-turbo のコンパイル時間とメモリ)。動かない場合は `ERR_SERVER_DIED` か 600 秒のタイムアウトになる。その場合の代替は処理デバイスを CPU / Vulkan にすること。
- Intel NPU ドライバ未導入だと検出行に「Intel NPU あり」が出ず、選んでもサーバが起動しない。

# v0.10.1 追加仕様: 無音・誤操作で「ご視聴ありがとうございました」が入力される問題

## 背景

- 右Alt(hold モード)を Alt+クリック等の**修飾キーとして**押すと録音が始まり、離した瞬間にクリック音だけの短い音声が STT に送られる。Whisper は無音・雑音に対して「ご視聴ありがとうございました」「ありがとうございました」「はい」などを幻聴し、それが貼り付いていた。

## 対策 (pipeline.rs)

1. **修飾キー利用のキャンセル(Windows)**: hold モードかつホットキーが単体修飾キー(`hook::is_special_token`)のとき、押下で `spawn_modifier_use_watch` を起動。キーが押されている間 20ms ごとに `GetAsyncKeyState` を見て、押下時点で押されていなかった**マウスボタンまたは修飾キー以外のキー**(VK 0x10-0x12 / 0xA0-0xA5 / 0xE7 / 0xFF 以外)が押されたら `cancel`(オーバーレイを閉じて idle、エラー表示なし)。キーを離した(タップロック/確定)・録音終了・キャンセル・10 秒経っても録音が始まらない、のいずれかで終了。Linux では無効。
2. **発話ゲート**: 16kHz サンプルを 20ms フレームに分け、ピークが full scale の 1.5% を超えるフレームの合計が **0.25 秒未満**なら STT に送らない。録音全体が 1.5 秒未満なら内部結果 `SILENT_SKIP`(status-changed は出さず、オーバーレイを閉じて idle)、1.5 秒以上なら従来の `ERR_NO_SPEECH`。
3. **幻聴フィルタ** `is_hallucination(text)`: 空白と句読点を除き小文字化した文字列が、空 / 全体が括弧(`[]` `()` `（）` `【】` `「」`)/ 40 文字以内で既知マーカー(「ご視聴」「チャンネル登録」「字幕提供」「thanksforwatching」「subtitlesby」「pleasesubscribe」「谢谢观看」「시청해주셔서」「untertitelvon」「soustitrespar」「subtítulospor」「legendaspela」「субтитрыот」「cảmơncácbạnđãtheodõi」「terimakasihtelahmenonton」「blank_audio」等 12 言語分。「字幕」「subscribe」「music」のような単語単体は正当な入力に出るので使わない)を含む、のいずれかなら真。最終結果なら `ERR_NO_SPEECH`、字幕ウィンドウならそのウィンドウを捨てる。**「ありがとうございました」「はい」単体は正当な入力なので対象外**(無音由来のものは 2 で止まる)。
- 単体テスト `gate_tests`(幻聴判定の正例/負例、発話ゲート)。

# v0.10.2 追加仕様: VAD と no_speech_prob による幻聴対策(v0.10.1 の見直し)

- v0.10.1 の「Alt 押下中のクリック/他キーで録音キャンセル」は**廃止**。Alt を押したままクリックで入力欄にカーソルを置いてから話す、という使い方を壊すため。発話ゲートと幻聴フィルタは維持。
- 参考: https://qiita.com/mtoyopet/items/407df09e6b2edfe35684 (`no_speech_prob` 0.5 判定 + 定型句 + 3 回以上の繰り返し)。

## サーバ側 VAD (setup.rs)

- whisper.cpp 内蔵の Silero VAD を使う。モデル `ggml-silero-v5.1.2.bin`(約 0.9 MB、`https://huggingface.co/ggml-org/whisper-vad/resolve/main/`)を `models/` に置く。`ensure_vad_model` が `sync_server_build`(起動・設定変更)と `quick_setup` の prewarm 直前に無ければ黙って取得(進捗イベント無し、失敗はログのみ)。
- 起動引数: **アプリ管理のサーバ**(server_path 空)でモデルがあるとき `--vad --vad-model <path> --vad-speech-pad-ms 120`。起動シグネチャに `vad=`。カスタム server_path には付けない(古いバイナリ対策)。
- 管理ビルドでも `--vad` を知らない古いビルドが即終了したら(`ERR_SERVER_DIED`)、`VAD_UNSUPPORTED` を立てて VAD 無しで即再起動(セッション中は VAD 無し)。

## no_speech_prob (stt.rs)

- ローカル/クラウドとも `response_format=verbose_json` で要求し、`stt::Transcript { text, no_speech }` を返す。`no_speech` = `segments` が 1 件以上あり、**全セグメント**の `no_speech_prob > 0.5`。`segments` の無い応答(OpenAI 互換の簡易実装)は false。
- 最終結果が `no_speech` なら `ERR_NO_SPEECH`。字幕ウィンドウが `no_speech` ならそのウィンドウを捨てる。`test_stt` は text を捨てる。
- **例外**: VAD 付きで起動した whisper-server(1.9.x)は、VAD が発話区間を 1 つも見つけなかった音声に `verbose_json` を返そうとして落ちる(無音・ノイズ・純音で再現。`json` なら `{"text":""}` を返す)。そのため `setup::server_vad_active()`(直近の起動が `--vad` 付きで成功したか)が真のときローカルは `json` で要求する。VAD が非発話を落とすので `no_speech_prob` は不要。VAD 無し(カスタム server_path / 古いビルド)とクラウドは `verbose_json`。
- 検証(2026-10-01、インストール済み ggml ビルド + tiny + Silero): 無音/ノイズ/純音 → VAD 0 区間 → `json` で空文字、サーバ生存。TTS 音声 → VAD 1 区間 → 文字起こし、`verbose_json` でも no_speech_prob 0.0 で生存。

## 繰り返し検出 (pipeline.rs)

- `is_hallucination` に `is_repetition`: 正規化後 6 文字以上で、同じ断片の 3 回以上の繰り返しだけで構成される(「ありがとうございました×3」「はいはいはいはいはい」)なら幻聴。「ははは」「はいはい、わかりました」は対象外。
- テスト追加(stt::tests、pipeline::gate_tests)。

# v0.10.3 追加仕様: 録音 HUD が描画されない不具合の対策

## 背景

- 2026-10-10: 約 31 時間常駐した v0.10.2 で、右Alt を押すと録音・文字起こし・貼り付けは動くが HUD が見えない状態になった。overlay ウィンドウは `IsWindowVisible=true` になり位置も画面内だったが、画面キャプチャでは中身が何も描画されていなかった(透明のまま)。WebView2 のプロセスは生存。スリープ復帰・GPU リセットのイベントは無し。アプリ再起動で直る。
- Tauri の `window.show()/hide()` はネイティブウィンドウの ShowWindow だけで、WebView2 の `ICoreWebView2Controller::SetIsVisible` は起動時から true のまま一度も切り替わらない。

## 対策 (pipeline.rs / overlay.ts)

1. `show_overlay` / `hide_overlay` は Windows で `with_webview` 経由でコントローラの `SetIsVisible(true/false)` も呼ぶ(wry の `set_visible` と同じ)。表示時は `NotifyParentWindowPositionChanged` も呼ぶ。
2. **描画ウォッチドッグ**: overlay は `status-changed{recording}` 受信後、`requestAnimationFrame` を 2 回待ってからコマンド `overlay_painted`(引数なし、戻り値なし)を invoke する。WebView2 が描画していないと rAF は走らない。`show_overlay` は表示ごとに連番を振り、**1.5 秒**以内にその表示以降の ack が無く、overlay がまだ表示中なら `fire-speak.log` に `overlay: no paint ack after show; recreating HUD window` を記録し、overlay ウィンドウを destroy して tauri.conf の定義から作り直す。作り直し後、status が idle 以外なら再表示する(作り直しは表示 1 回につき 1 回まで)。
3. overlay は起動時に `get_status` を読み、idle 以外ならその状態を表示する(作り直された HUD が録音途中から始まるため)。
- 検証(2026-10-10、debug ビルド + WebView2 リモートデバッグ): 通常の録音で `visibilityState=visible`・rAF 約 60fps・ack 到着(作り直し無し)。HUD の `requestAnimationFrame` を無効化して録音 → 1.5 秒後にログ記録・新しい overlay ウィンドウで `recording` 表示・キャンセルで非表示。作り直し後の録音は正常。

# v0.11.0 追加仕様: 文脈による語の選択 (context.rs)

- 設定 `llm_context: bool`(既定 true、一般 → 貼り付けカードのスイッチ「文脈で語を選ぶ」)。true なら LLM モード(整形ほか全 LLM モード)の整形時に参考文脈をシステムプロンプト末尾へ付ける。ユーザーメッセージは従来どおり文字起こしのみ。
- **クリップボード履歴**: 起動時に監視スレッドを開始し、1 秒ごとにクリップボードのテキストを確認、変化したら記録(新しい順・最大 10 件・同一テキストは先頭へ移動)。Windows は `GetClipboardSequenceNumber` が変わったときだけ開く。**メモリ内のみ**(ディスクに書かない)。`llm_context` が false の間は記録せず履歴を消去。アプリ自身の貼り付け・コピー・復元は記録しない(`paste.rs` が `mark_own_write`、次の確認でその内容を既読扱い)。
- 参考文脈:
  - **直近の音声入力**: 履歴の新しい順に 30 分以内・最大 3 件の `final_text`(各末尾 600 字)、古い順に並べる。
  - **クリップボード履歴**: 60 分以内・最大 5 件(各末尾 800 字、合計 3000 字、新しいものから優先)、古い順に並べる。整形直前(貼り付け前)に読んだ現在のテキストも合流(1 秒未満のコピー取りこぼし対策)。直近 20 件の音声入力履歴の `final_text` と同一のもの(=自分の出力)は除外。空白を含まない 12 字以上の ASCII で英字と非英字が混在するもの(パスワード・API キー形)は記録も送信もしない(`://` を含む URL は除く)。
- 指示: 参考文脈は同音異義語・固有名詞・専門用語・表記ゆれの判断材料にのみ使い、出力に含めない・内容を補わない・無関係なら無視。**クリップボード履歴は正確な表記として最優先**(読み・音が同じか近い語があればその表記を採用)。直近の音声入力は誤認識を含み得るため話題把握の補助のみ。プロンプト内の順序もクリップボード履歴 → 直近の音声入力。
- **STT プロンプト(語彙ヒント)**: `llm_context` が true なら、クリップボード履歴(60 分以内)から用語を抽出して whisper の `prompt` に渡す(ローカル `/inference`・クラウド `/audio/transcriptions` とも `prompt` フィールド)。LLM を使わないモード・「なし」でも有効。
  - 抽出: 漢字・カタカナの 2 字以上の連続(ひらがなは区切り、`・` も区切り)、英字を含む 2 字以上の ASCII 語(`.` `-` `_` `+` `#` を含めてよい、例 `Next.js` `fire-speak` `C#`)。小さなストップワード表(the, and…)は除外。URL(`://`)・メールアドレス(`@`)・秘密情報形の ASCII 連続は丸ごと除外。
  - 上限: 合計 100 字・30 語(whisper はプロンプトの末尾約 224 トークンしか使わず、漢字は 2〜3 トークン)。重複除去。並びは古いコピー → 新しいコピー(末尾ほど残るため最新を最後に)。区切りは認識言語が ja/zh なら `、`(末尾 `。`)、それ以外は `, `(末尾 `.`)。プロンプトの句読点が出力に移るため。
  - **最終認識のみ**。リアルタイム字幕・プリウォーム・接続テストには付けない(短い窓ほど復唱しやすい)。
  - **復唱検出**: 結果がプロンプト全体と一致、または 3 語以上のプロンプト用語だけで構成される(英数字以外を除去・小文字化して比較)なら、プロンプト無しで **1 回だけ** 再認識し、その結果を通常の幻聴判定に回す。1〜2 語だけの発話(「講演会」)は有効。
- テスト: context::tests。

# v0.12.0 追加仕様: Qwen3-ASR (ローカル llama-server / クラウド OpenRouter)

## エンジン "qwen" (qwen.rs)

- `stt.engine` に `"qwen"` を追加(ローカル Qwen3-ASR)。設定 `stt.qwen { model: "0.6b"|"1.7b"(既定 0.6b), server_port: 8179, threads(既定は whisper と同じ), accel: "auto"|"gpu"|"cpu" }`。
- 実行: llama.cpp の `llama-server` に ggml-org の GGUF 変換(`ggml-org/Qwen3-ASR-{0.6B,1.7B}-GGUF` の Q8_0、モデル + mmproj)を読ませる。`POST /v1/chat/completions` に `{"messages":[system?, {"role":"user","content":[{"type":"input_audio","input_audio":{"data":<WAV base64>,"format":"wav"}}]}, assistant?], "temperature":0, "max_tokens":4096}`。
  - 出力は `language Japanese<asr_text>本文`。`<asr_text>` より後を本文とし、言語タグ `None` は `no_speech`(`qwen::parse_asr_output`。クラウド応答にも同じ処理をかける)。
  - 認識言語が固定(≠auto)なら assistant ターンに `language {英語名}<asr_text>` を prefill して言語を固定する。
  - 語彙ヒント(v0.11 のクリップボード用語)は **system メッセージ**として渡す(Qwen3-ASR の文脈入力)。
- 配布物はピン留め+SHA-256 検証:
  - llama.cpp `b11684` の Vulkan ビルド(Windows `llama-b11684-bin-win-vulkan-x64.zip` 約 30MB、Linux `llama-b11684-bin-ubuntu-vulkan-x64.tar.gz`)。CPU バックエンドも同梱(ggml は実行時ロード)なので GPU/CPU 両用。置き場所は `{app_data}/llama/`(`bin/` は whisper 用で `*server*.exe` を再帰検索するため、`ggml-rpc-server.exe` を含む llama.cpp は別フォルダ)。`llama.new/` に展開・検証してから入れ替え。
  - モデルは Hugging Face のコミット固定 URL から `{app_data}/models/` へ(mmproj → モデルの順、各 `.part` → 検証 → rename)。0.6B = 972MB、1.7B = 2404MB。
- 起動: `llama-server -m <model> --mmproj <mmproj> --host 127.0.0.1 --port <port> -t <threads> -c 8192 -np 2 --no-webui`、CPU のとき `-dev none`。ヘルスは `GET /health` が 200(読み込み中は 503 なので TCP 接続では判定しない)、上限 120 秒。Job Object / PDEATHSIG は whisper と共通(`setup::spawn_bound`)。シグネチャ(exe・モデル・ポート・スレッド・accel)が変われば再起動。
- `auto` は NVIDIA または AMD Radeon(`hw::Hardware`)があれば GPU、それ以外(iGPU のみ・GPU なし)は CPU。
- エンジン切替時: qwen 以外になれば llama-server を、local 以外になれば whisper-server を停止。アプリ終了経路はすべて両方停止(`kill_engines`)。起動時・設定変更時は `qwen::prewarm`(起動 + 1 秒の無音を認識して Vulkan のシェーダコンパイル約 7 秒を先に済ませる)。
- 未インストールで認識要求 → `ERR_SETUP_REQUIRED`。ホームのワンクリックセットアップは engine=qwen なら llama-server → 選択モデルの順に入れる(`quick_setup` が分岐)。
- コマンド: `qwen_download_server`、`qwen_download_model { model }`。進捗イベントは `download-progress` の kind `qwen-server` / `qwen-model`(name はモデル id。mmproj とモデルでバーが 2 回進む)。`SetupStatus.qwen: { server_installed, server_path, model_installed, models[{id, installed, size_mb}], server_running, effective_accel }`。
- UI: エンジン選択に「ローカル (Qwen3-ASR)」。パネルは llama-server カード(状態・パス・インストール・処理デバイス 自動/GPU (Vulkan)/CPU)、モデル選択(0.6B / 1.7B)、詳細設定(ポート・スレッド)。ホームの状態カードはエンジン(llama.cpp · デバイス · 稼働状態)とモデル。
- 実測(2026-10-16、RTX 4070 Ti SUPER、0.6B Q8_0、TTS 音声): Vulkan で 6〜8 秒の発話 0.1〜0.35 秒(初回のみ約 7 秒)、CPU 4 スレッド(`-dev none`)で 0.3〜0.6 秒。76 秒の連続発話も欠落なし(プロンプト約 13 トークン/秒)。system に「fire-speak」を入れると「ファイアスピーク」→「fire-speak」。無音は「language Portuguese…」等を幻聴するので、既存の発話量ゲート(`MIN_SPEECH_SECS`)と言語 prefill が前提。

## クラウド: OpenRouter プリセット (stt.rs)

- クラウド STT のプリセットに `OpenRouter (Qwen3-ASR)` = `https://openrouter.ai/api/v1` / `qwen/qwen3-asr-0.6b`(OpenRouter の `/audio/transcriptions` は OpenAI 形式の multipart `file` を受ける。提供元 DeepInfra、約 $0.0002/分)。
- `prompt` と `response_format=verbose_json` を受け付けない OpenAI 互換エンドポイントのため、**HTTP 400 なら 1 回だけ** `file` / `model` / `language` のみで再送する。
- 未検証: OpenRouter 実機での応答(API キーが無く未確認)。

# v0.12.1 追加仕様: アップデートの通知を目立たせる

- 背景: 更新確認は起動時 1 回だけで、通知はメインウィンドウ内(バナー/ダイアログ)のみだった。fire-speak は Windows 起動時に `--hidden` で常駐し何日も動くため、常駐中に出たリリースも、ウィンドウを開かない限り見逃していた。
- バックエンドの定期確認(`update::start_background_checks`): `update.auto_check` が true の間、起動 20 秒後と以後 **6 時間ごと**に確認。設定画面の「更新を確認」も同じ `update::check_now` を通る。新しい版が見つかったら `update::AVAILABLE` に保持し、次をすべて行う:
  1. **デスクトップ通知**(tauri-plugin-notification、版ごとに 1 回): タイトル「fire-speak v{版} が出ました」、本文「トレイアイコンのメニューか設定画面からアップデートできます」(12 言語、`locale::update_labels`)。
  2. **トレイ**: メニュー先頭に「⬆ v{版} にアップデート」(押すとメインウィンドウを開き、イベント `update-open-dialog` で更新ダイアログを表示)+区切り線。ツールチップ「fire-speak — v{版} available」。
  3. イベント `update-available`(UpdateInfo)。設定ウィンドウはバナー・ダイアログ(版ごとに 1 回)に加え、サイドバー「アップデート」に点滅バッジ `v{版}`(閉じられない)。HUD は保持する。
- **HUD**: 新しい版がある間、音声入力の完了(done)時に字幕欄へ金色で「⬆ v{版} が出ています — トレイのメニューからアップデート」(`overlay.updateAvailable`、12 言語)。このとき done の表示時間を 1.2 秒 → 3.5 秒に延ばす。
- コマンド `get_available_update` → `UpdateInfo | null`(後から開いたウィンドウ・HUD が既知の結果を読む)。
- 検証(2026-10-16): version 0.11.0 に偽装した debug ビルドを `--hidden` 起動 → 20 秒後に GitHub を確認し、Windows の通知 DB に「fire-speak v0.12.0 が出ました」のトーストが届いた(未パッケージの debug ビルドなので送信元は PowerShell 名義。インストール版はアプリ名義の想定、未確認)。トレイメニュー・HUD・バッジは画面で未確認。

# v0.12.2 追加仕様: Qwen3-ASR を ARM 機でも / 処理デバイスの整理

- llama-server の配布物に **Windows arm64**(`llama-b11684-bin-win-vulkan-arm64.zip`)と **Linux arm64**(`llama-b11684-bin-ubuntu-vulkan-arm64.tar.gz`)を追加。どちらも Vulkan 版で CPU バックエンド同梱(`-dev none` で CPU)。それまで x64 以外では `ERR_NO_SERVER_ASSET` だった。
- 処理デバイスは引き続き 自動 / GPU (Vulkan) / CPU。表示名を「GPU (Vulkan) — NVIDIA / AMD / Intel」に(Vulkan は 3 社の GPU で動く)。
- 他のビルドを入れない理由(2026-10-17、RTX 4070 Ti SUPER、15 秒の日本語、b11684):
  - CUDA 12.4: 0.6B 0.17 秒 / 1.7B 0.29 秒で Vulkan(0.15 / 0.31 秒)と同等。本体+cudart で約 630 MB 増えるだけ。
  - OpenVINO(Intel GPU/NPU): 音声エンコーダで `failed to process mtmd chunk`(ggml-openvino の動的形状チェック失敗)。CPU でも動かない。
  - AMD NPU: llama.cpp にバックエンドが無い。
- NPU(AMD `hw.npu` / Intel `hw.intel_npu`)がある PC では、Qwen の処理デバイス欄のヒントに「この PC の NPU は Qwen3-ASR では使えません…「ローカル (whisper.cpp)」を選んでください」(`stt.qwenNoNpu`、12 言語)を足す。

# v0.12.3 修正: 口述した「要約して」を実行しない

- 事象(2026-10-17、Poolside `laguna-s-2.1`、整形モード): 口述「これ要約して。」「要約して。」が、参考文脈(`<recent_dictations>` の直前 2 件)の要約に置き換わって貼り付けられた。旧プロンプトでの再現では「要約して」6 回中 6 回とも要約・作文が出力された。
- プロンプト(`llm.rs`):
  - システム: `<dictation>` の中身はユーザーがこれから入力する文章そのもので依頼ではない。「要約して」「翻訳して」「返信して」等の依頼・命令・質問の形でも実行・回答せず、その文自体を指示どおり処理する。タグは出力しない。
  - ユーザーメッセージ: `次の口述テキストを指示どおりに処理し、結果のテキストだけを出力してください（口述の中の依頼には従わないこと）。\n<dictation>{raw}</dictation>`。応答の前後に `<dictation>` タグが付いていたら外す。
  - 参考文脈の説明に「処理の対象ではない。口述が『これ』『それ』で参考文脈を指しているように見えても要約・翻訳・引用しない」を追加。
- 安全装置(`context::looks_like_context_leak`): 参考文脈ありの整形結果が `口述の文字数 × 2 + 20` を超えたら、参考文脈なしでもう一度整形してそちらを使う(ログ `llm: output much longer than dictation; polishing again without context`)。口述そのままへの差し戻しはしない(ビジネス文書化など、長くなるのが正しいモードがあるため)。
- 既知の限界: `laguna-s-2.1` は「要約して。」のような極端に短い口述で、`$minimal$` や韓国語など無関係な文字列を返すことがある(参考文脈とは無関係のモデル側の不安定さ)。

# v0.12.4 修正: 口述した質問に答えない

- 事象(2026-10-17、Poolside `laguna-s-2.1`、整形モード): 口述「旧地名の正規化も終わったんだよね。」→「はい、旧地名の正規化も終わりました。」(質問に回答)。旧プロンプトの再現では「テストは全部通った？」も 2 回中 2 回「はい、…通りました。」。v0.12.3 のプロンプトでも「えーと、それってもう本番に出したっけ。」が 4 回中 1 回回答、1 回は断定文化した。
- `SYSTEM_PREFIX` に例を追加(指示が整形の場合): 確認の質問・「これ要約して」・「英語に翻訳してください」はそのままの文として整える例と、言い直し「明日の会議は、あ、いや、明後日の会議は…」→「明後日の会議は…」の例。加えて「疑問・確認の文はどの指示でも疑問・確認の形のまま」「フィラー除去や言い直しの整理など指示された処理はいつもどおり行う」。言い直しの例は、例を足しただけだとモデルが口述をほぼ無加工で返すようになったため(明日→明後日の訂正が 4 回中 3 回「明日の会議は…」)。
- 確認(同モデル、各 3〜4 回): 質問・依頼形の口述 6 種は全回そのまま。言い直しの例は 3/3。英訳モードは「これ要約して。」→「Please summarize this.」。

# v0.13.0 追加仕様: 用語辞書(自動で育つ)と用語置換パス

## 用語置換パス (llm::fix_terms)

- 背景(2026-10-17): クリップボード「Qwen3-ASR」で口述 →「クエン三ASRを推奨にしたい。」のまま。Poolside `laguna-s-2.1` は整形プロンプト内のクリップボード文脈から用語を拾えず(3 回中 1 回、用語リストを足しても 4 回中 2 回)。
- 整形のあと、置換だけをする短い LLM 呼び出しを追加。system は `TERM_FIX_SYSTEM`(用語の読み上げに聞こえる部分だけを用語の表記に置換、他は一文字も変えない、例 3 つ)、user は `用語: A / B\n文: {整形結果}`。単独なら同モデルで 4/4。
- 実行条件(`context::term_fix_candidates`): クリップボード由来の用語のうち英字かカタカナを含み、まだ文中に無いもの。文にカタカナ 2 文字以上の並びか英字が無ければ呼ばない(1 回 1〜4 秒かかるため。漢数字だけでは呼ばない)。
- 採用条件(`context::accept_term_fix`): 新たに用語が丸ごと入り、その用語を除いた残りの 80% 以上が元の文と順序どおり一致(LCS)。中途半端な置換(「クエン3-ASR」「クエン３－ＡＳＲ」)や書き換えは捨てる。

## 用語辞書 (dictionary.rs)

- 保存先 `{app_config_dir}/dictionary.json`。`DictEntry { term, aliases[], source: "user"|"auto", hits, added, last_used }`。
- 自動学習(`llm_context` が on のとき、毎回の音声入力の後):
  - クリップボード由来の用語(英字かカタカナを含む 3〜40 文字)が最終テキストに入った → `auto` で追加/更新。漢字だけの語は学習しない(一般語が増えるため)。
  - 用語置換パスが採用された → その用語を追加し、置き換えられた元の表記を **alias** として保存(`extract_alias`: 用語の前後が変わっていないときだけ。例「クエン三ASR」)。alias は 1 語 8 個まで。
  - 辞書の語が最終テキストに出た → `hits` / `last_used` 更新。
  - `auto` は 300 件まで(古い順に削除)、180 日使われなければ削除。`user` は消えない。
- 利用(常に。学習の on/off とは別):
  - STT プロンプト: クリップボード用語の残り枠(100 文字 / 30 語)に辞書の語を入れる。順序は辞書(優先度の低い順)→ クリップボード(whisper は末尾を残す)。
  - alias 置換: STT 結果と整形結果の両方に、alias → 用語の文字列置換(長い alias 優先)。LLM 不要・遅延なし。
  - 整形プロンプト: 参考文脈に `<terms>` として最大 40 語(user 優先、次に最近使った順)。
- 設定画面「用語辞書」(サイドバー 📖): 用語と「誤認識・読み」(カンマ区切り)を追加、一覧(用語・手動/自動・回数・誤認識の表記)、削除。コマンド `dict_list` / `dict_add { term, aliases }` / `dict_remove { term }`、イベント `dictionary-updated`。ロケール 12 言語(`nav.dictionary`, `dict.*`, `msg.ERR_DICT_TERM`)。

## 参考: 日本語の認識精度 (2026-10-17、RTX 4070 Ti SUPER)

- Windows 音声合成 4 声で 8 文(200 文字、数字・同音異義語・地名入り)。句読点を除き、数字表記の違い(三時/3時)は同一視した文字誤り率: whisper large-v3-turbo 0.5%、Qwen3-ASR 1.7B 1.5%(丸ノ内・子どもなど表記ゆれが中心)、0.6B 4.5%(千代田区→都心区など)。合成音声なので実際の話し声とは差がある。既定の推奨エンジンは whisper のまま。

# v0.14.0 追加仕様: Parakeet 日本語 / Cohere Transcribe (ローカル CrispASR / クラウド Cohere)

## 調べたこと (2026-10-17)

- **Parakeet 日本語** = `nvidia/parakeet-tdt_ctc-0.6b-ja`(FastConformer + TDT/CTC、0.6B、CC-BY-4.0、日本語専用、句読点あり)。公式は NeMo(PyTorch)のみ。GGUF 変換は `cstr/parakeet-tdt-0.6b-ja-GGUF`(CrispASR 用)、`kun432/parakeet-cpp-ja-gguf`(mudler/parakeet.cpp 用)ほか、ONNX・CoreML・MLX 版もある。クラウド API は NVIDIA NIM(gRPC、OpenAI 互換ではない)のみ。
  - 手元の `%APPDATA%\jp.firemio.fire-speak\bin\Release\parakeet-cli.exe` は whisper.cpp 1.9.4 の CUDA ビルドに同梱の parakeet 実装で、既定モデルは多言語の `ggml-parakeet-tdt-0.6b-v3.bin`(日本語非対応、サーバーモード無し)。日本語版には使わない。
- **Cohere Transcribe** = `CohereLabs/cohere-transcribe-03-2026`(Conformer エンコーダ + Transformer デコーダ、2B、Apache-2.0、日本語を含む 14 言語、言語自動判定なし)。HF の元リポジトリは利用規約への同意が必要(gated)だが、GGUF 変換 `cstr/cohere-transcribe-03-2026-GGUF`・`handy-computer/cohere-transcribe-03-2026-gguf` は同意不要で Apache-2.0。クラウドは Cohere API(`POST /v2/audio/transcriptions`、OpenAI 互換は `/compatibility/v1/audio/transcriptions`、`language` 必須、`file` はフォームの最後、`response_format` は json のみ、`prompt` 非対応、25MB まで。無料枠はレート制限付き)。OpenRouter には無い。
- ローカル実行手段: どちらも動かせ、Windows/Linux のビルド済みバイナリとサーバーモードがあるのは **CrispASR**(CrispStrobe/CrispASR、MIT、ggml、`crispasr --server` が OpenAI 互換の `/v1/audio/transcriptions`)。transcribe.cpp(handy-computer)は Cohere は動くが Parakeet 日本語版が対象外、parakeet.cpp は Cohere が対象外のため、1 つで両方を賄える CrispASR を採用。

## エンジン "parakeet" / "cohere" (crisp.rs)

- `stt.engine` に `"parakeet"`(Parakeet TDT-CTC 0.6B ja)と `"cohere"`(Cohere Transcribe 03-2026)を追加。どちらも CrispASR で動かし、設定は共通の `stt.crisp { server_port: 8180, threads(既定は whisper と同じ), accel: "auto"|"gpu"|"cpu" }`。モデルはエンジンで決まる(選択肢なし)。
- 配布物はピン留め+SHA-256 検証:
  - CrispASR `v0.8.43`: Windows x64 `crispasr-windows-x86_64-vulkan.zip`(36MB)、Linux x64 `crispasr-linux-x86_64-vulkan.tar.gz`、Linux arm64 `crispasr-linux-arm64.tar.gz`(CPU のみ)。Windows arm64 用は配布が無く `ERR_NO_SERVER_ASSET`。置き場所 `{app_data}/crispasr/`(`crispasr.new/` に展開・検証してから入れ替え)。
  - モデル(Q8_0、HF コミット固定 URL → `{app_data}/models/`): Parakeet `cstr/parakeet-tdt-0.6b-ja-GGUF` の `parakeet-tdt-0.6b-ja-q8_0.gguf`(643MB)、Cohere `cstr/cohere-transcribe-03-2026-GGUF` の `cohere-transcribe-q8_0.gguf`(2312MB)。Parakeet の Q4_K は TDT デコードがループするため使わない。
- 起動: `crispasr --server -m <model> --host 127.0.0.1 --port <port> -t <threads>`、CPU のとき `-ng`、Linux は `LD_LIBRARY_PATH` に exe のフォルダ(同梱の libopenblas 等)。ヘルス `GET /health` 200、上限 120 秒。シグネチャ(exe・モデル・ポート・スレッド・accel)が変われば再起動するので、parakeet ⇄ cohere の切替でモデルを読み直す。`auto` は Qwen と同じ(NVIDIA / AMD Radeon があれば GPU)。
- 認識: `POST /v1/audio/transcriptions`(multipart: `response_format=json`、`temperature=0`、認識言語が auto 以外なら `language`、`file`)。**語彙ヒント(prompt)は送らない**(両モデルとも CrispASR では無視され、結果が変わらないことを確認)。用語辞書の alias 置換と LLM の用語置換パスは従来どおり効く。無音は「どうぞ。」等を出すことがあるので、既存の発話量ゲート(`MIN_SPEECH_SECS`)が前提。
- エンジン切替時: parakeet/cohere 以外になれば CrispASR を停止。終了経路は全サーバー停止(`kill_engines`)。起動時・設定変更時・ダウンロード後は `crisp::prewarm`(1 秒の無音を認識)。
- ワンクリックセットアップは CrispASR → エンジンのモデルの順。コマンド `crisp_download_server`、`crisp_download_model { model: "parakeet"|"cohere" }`。進捗イベント kind `crisp-server` / `crisp-model`。`SetupStatus.crisp: { server_installed, server_path, models[{id, installed, size_mb}], server_running, effective_accel }`。
- UI: エンジン選択に「ローカル (Parakeet 日本語)」「ローカル (Cohere Transcribe)」。パネル(両エンジン共通)は CrispASR カード(状態・パス・インストール・処理デバイス)、モデル(エンジンのモデル 1 件・ダウンロード・ライセンス等のヒント `stt.parakeetHint` / `stt.cohereHint`)、詳細設定(ポート・スレッド)。ホームの状態カードはエンジン(CrispASR · デバイス · 稼働状態)とモデル。ロケール 12 言語(`stt.engineParakeet`, `stt.engineCohere`, `stt.crispModel`, `stt.parakeetHint`, `stt.cohereHint`, `setup.crispServerInstalled`)。

## クラウド: Cohere プリセット (stt.rs)

- プリセット `Cohere Transcribe` = `https://api.cohere.com/compatibility/v1` / `cohere-transcribe-03-2026`。
- multipart の `file` を常に**最後**に送る(Cohere の要件。OpenAI・Groq・OpenRouter にも影響なし)。初回の `verbose_json` が 400 なら従来どおり file/model/language のみで再送。Cohere は `language` 必須なので、認識言語「自動」では失敗する。
- 未検証: Cohere API 実機での応答(API キーが無く未確認)。

## 参考: 日本語の認識精度 (2026-10-17、RTX 4070 Ti SUPER)

v0.13.0 と同じ 8 文(Windows 音声合成 4 声、200 文字)・同じ正規化。処理時間は 8 文の合計(各サーバー起動・ウォームアップ後、1 文 3〜6 秒の音声)。

| エンジン | 文字誤り率 | 処理時間 (GPU) | 処理時間 (CPU 8 スレッド) | 主な誤り |
| --- | --- | --- | --- | --- |
| whisper large-v3-turbo (whisper.cpp CUDA) | 0.5% | 0.87 秒 | — | — |
| Qwen3-ASR 1.7B (llama.cpp Vulkan) | 1.5% | 1.16 秒 | — | 丸ノ内・子ども |
| **Parakeet TDT-CTC 0.6B ja** (CrispASR Vulkan) | **2.0%** | **0.87 秒** | 2.56 秒 | 売り上げ・子ども(表記ゆれのみ) |
| Qwen3-ASR 0.6B (llama.cpp Vulkan) | 4.5% | 0.74 秒 | — | 千代田区→都心区 |
| **Cohere Transcribe 03-2026** (CrispASR Vulkan) | **7.0%** | 1.15 秒 | 7.01 秒 | 明日→あした、見積書→書(冒頭欠落)、整形→成型、東京都→東京・ |

- Parakeet は誤りが送り仮名の表記ゆれだけで、CPU でも 1 文約 0.3 秒と軽い。Cohere は冒頭の語を落とすことがあり、日本語では他より劣る。合成音声なので実際の話し声とは差がある。既定の推奨エンジンは whisper のまま。
- 参考: Cohere の日本語追加学習版 `TransWithAI/cohere-transcribe-ja-v0.1-GGUF` は 5.0% だが、ライセンス表記が無いため入れていない。

# v0.14.1 追加仕様: エンジン選択に精度と速度を表示

- 背景: エンジンが 5 つになり、ボタン名だけでは何を選べばよいか分からない。
- `#stt-engine-seg` をグリッド(1 列 210px 以上)にし、各ボタンの 2 行目に実測値 `誤り {文字誤り率} · GPU {秒} · CPU {秒}`(`stt.engineMeta`)。クラウドは「精度・速度はサービス次第」(`stt.engineMetaCloud`)。値は main.ts の `ENGINE_METRICS`(v0.14.0 のベンチマーク、1 文 約 4 秒あたり)。下に測定条件とおすすめ(GPU あり → whisper.cpp、GPU なし → Parakeet)の注記(`stt.engineNote`)。12 言語。
- CPU の値は Ryzen 7 9800X3D・8 スレッドで追加測定(8 文合計): whisper large-v3-turbo 43.6 秒、Qwen3-ASR 1.7B 10.9 秒、0.6B 4.3 秒、Parakeet 2.6 秒、Cohere 7.0 秒。1 文あたり 5.5 / 1.4 / 0.5 / 0.3 / 0.9 秒。古い CPU(第 8 世代 Core i5 など)は数倍かかる見込み(未計測)。

# v0.14.2 追加仕様: エンジン選択を比較表に / この PC へのおすすめ

- 背景: 数値の文字表示だけでは比べにくい。環境(古めのノート PC など)から選べるようにしたい。
- `#stt-engine-seg` を比較表のカードにした。行は whisper.cpp large-v3-turbo / Qwen3-ASR 1.7B / Parakeet 0.6B ja / Qwen3-ASR 0.6B / Cohere 2B(`ENGINE_CHOICES`)+ クラウド(常に最後、数値なし)。各行: ラジオ・名前・向いている環境(`stt.fit*`)・誤り率の棒・CPU 処理時間の棒(どちらも最大値に対する線形、短いほど良い)・ダウンロード量。GPU ではどれも 0.1〜0.15 秒なので棒は CPU のみ。
- 並び順: 精度順(既定)/ 速さ順 (CPU) / 軽さ順。セッション内のみ保持。
- 行のクリックでエンジンを選ぶ。Qwen の行は `stt.qwen.model` も切り替える(未インストールならホームのセットアップがそのモデルを入れる)。
- おすすめ(`recommendedChoice`): NVIDIA / AMD Radeon があれば whisper.cpp、無ければ認識言語が日本語か自動なら Parakeet、それ以外は Qwen3-ASR 0.6B。その行に「この PC におすすめ」バッジ、表の下に「この PC({GPU 名 / GPU なし})には {名前} がおすすめです。」+ 測定条件。
- ロケール 12 言語(`stt.sortBy`, `stt.sort*`, `stt.col*`, `stt.seconds`, `stt.recommended`, `stt.fit*`, `stt.recoThisPc`, `stt.engineNote`)。v0.14.1 の `stt.engineMeta` は削除。
- 確認: 静的モック(Tauri IPC スタブ)で GPU なし / NVIDIA の表示、並び替え、Qwen 0.6B 行の選択でパネルが Qwen に切り替わることを確認。

# v0.14.3 変更: 使い道のない whisper モデルを外す / CPU だけの PC は Parakeet から

- 計測(v0.14 と同じ 8 文、1 文あたり、GPU = RTX 4070 Ti SUPER / CPU = Ryzen 7 9800X3D 8 スレッド): whisper tiny 誤り 24.5%・0.06 / 0.19 秒、base 12.0%・0.08 / 0.39 秒、small 8.5%・0.12 / 1.3 秒。日本語では Parakeet(2.0%・CPU 0.32 秒)が精度・速さとも上回り、medium は large-v3-turbo に劣る。tiny は「東京都千代田区丸の内一丁目」→「東京都知用だくなるのをちい調明に」で実用外。
- whisper のダウンロード対象は large-v3-turbo のみ(`setup::DOWNLOADABLE`)。tiny / base / small / medium は `MODELS` に残し、インストール済みなら従来どおり一覧に出て選べる・使える(既存環境を壊さない)。`download_model` は対象外を `ERR_INTERNAL` で拒否。`recommended_model` は常に large-v3-turbo。
- 初回セットアップ(ホームのカード): engine = local・whisper のサーバーもモデルも未導入・モデルパス上書きなし・処理デバイスが CPU のとき、比較表のおすすめ(`recommendedChoice`)に切り替えて案内する(`onboardRedirect`)。日本語/自動 → Parakeet(CrispASR が無い Windows arm64 では Qwen3-ASR 0.6B)、それ以外の言語 → Qwen3-ASR 0.6B。「セットアップを開始」で `stt.engine`(と Qwen のモデル)を保存してから `quick_setup`。おすすめ先がすでに入っている場合は切り替えない。
- `SetupStatus.crisp.available`: この OS/CPU 向けの CrispASR があるか。
- 比較表: 並び順に使っている列の見出しを強調(▲ + アクセント色)。

# v0.15.0 UI 整理: サイドバー 6 項目・設定の集約

- サイドバー: ホーム / 音声認識 / AI整形 / 用語辞書 / 履歴 / 設定(8 → 6)。「一般」は「設定」に改名(`nav.general`、section id は `general` のまま)し、「アップデート」を統合(更新バッジ `nav-update-badge` は設定に付く)。「モード」は AI整形 ページの後半へ。
- 設定ページのカード: ホットキー / 貼り付け(方法・クリップボード復元)/ 動作(文脈で語を選ぶ・リアルタイム字幕)/ システム(表示言語・自動起動・設定フォルダを開く)/ アップデート(現在の版・更新を確認・自動確認)。アップデートの GitHub オーナー / リポジトリ欄は画面から削除(設定値 `update.owner/repo` は残る)。
- 認識言語は音声認識ページの先頭へ(ホームの状態行「認識言語」の「変更」も音声認識へ)。履歴の保存件数は履歴ページの見出し行へ。
- AI整形: 見出し → プロバイダ一覧 → その下に「＋ 追加: Poolside / Claude / OpenAI / OpenRouter / Groq / Ollama / カスタム」の 1 行(旧「＋ プロバイダを追加」ボタンとプリセット行・説明文を統合)。モードも一覧の下に「＋ モードを追加」。
- whisper のモデル選択: 選べるモデルが 1 つ(large-v3-turbo のみ)なら「自動」行を出さない。インストール済みモデルの再ダウンロードボタンは削除。
- ホーム: ヒーローの下に 1 行、`llm_context` オン時「📋 音声入力のたびに、クリップボードの直近の内容と直前の入力を参考にして用語や同音異義語を判断します。コピーした用語は用語辞書に自動で追加されます。」/ オフ時「…オフです。」+「変更」(設定へ)。`home.contextOn/Off`。
- README: 用語辞書が自動で育つ仕組みを記載。
- ロケール 12 言語(`nav.general` の訳を「設定」に、`general.behaviorHeading`、`home.context*`、`llm.addLabel`、`llm.custom`)。
- 確認: 静的モック(Tauri IPC スタブ)で全 6 ページの表示・遷移、コンソールエラーなし。
