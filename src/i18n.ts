// fire-speak — i18n runtime (SPEC v0.2)
//
// Flat key/value dictionaries in src/locales/{code}.json. ja.json is the
// master key set; every locale must carry the identical keys. Placeholders
// are {0}, {1}, ... Backend-originated messages use the closed "KEY|p0|p1"
// format and are rendered via tMsg().

import ja from "./locales/ja.json";
import en from "./locales/en.json";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";
import ko from "./locales/ko.json";
import es from "./locales/es.json";
import fr from "./locales/fr.json";
import de from "./locales/de.json";
import ptBR from "./locales/pt-BR.json";
import ru from "./locales/ru.json";
import vi from "./locales/vi.json";
import id from "./locales/id.json";

export interface LangMeta {
  code: string;
  /** Native display name, shown as-is in the language selector (SPEC). */
  native: string;
}

/** The 12 supported UI languages (SPEC order). */
export const LANGS: readonly LangMeta[] = [
  { code: "ja", native: "日本語" },
  { code: "en", native: "English" },
  { code: "zh-CN", native: "简体中文" },
  { code: "zh-TW", native: "繁體中文" },
  { code: "ko", native: "한국어" },
  { code: "es", native: "Español" },
  { code: "fr", native: "Français" },
  { code: "de", native: "Deutsch" },
  { code: "pt-BR", native: "Português (Brasil)" },
  { code: "ru", native: "Русский" },
  { code: "vi", native: "Tiếng Việt" },
  { code: "id", native: "Bahasa Indonesia" },
];

type Dict = Record<string, string>;

const DICTS: Record<string, Dict> = {
  ja,
  en,
  "zh-CN": zhCN,
  "zh-TW": zhTW,
  ko,
  es,
  fr,
  de,
  "pt-BR": ptBR,
  ru,
  vi,
  id,
};

const FALLBACK: Dict = ja;

let currentLang = "ja";

/** Substitute {0}, {1}, ... in a template. Unknown indices are left as-is. */
function substitute(template: string, params: readonly (string | number)[]): string {
  return template.replace(/\{(\d+)\}/g, (match, idx: string) => {
    const value = params[Number(idx)];
    return value === undefined ? match : String(value);
  });
}

/**
 * Switch the active UI language. Unknown codes fall back to "en" (the backend
 * always resolves "" to a concrete code before the frontend sees it, so this
 * is purely defensive). Also updates <html lang>.
 */
export function setLang(code: string): void {
  currentLang = code in DICTS ? code : "en";
  document.documentElement.lang = currentLang;
}

export function getLang(): string {
  return currentLang;
}

/**
 * Translate a UI key. Missing key: fall back to the ja value, then to the key
 * itself.
 */
export function t(key: string, ...params: (string | number)[]): string {
  const raw = DICTS[currentLang][key] ?? FALLBACK[key] ?? key;
  return params.length > 0 ? substitute(raw, params) : raw;
}

/** Special single-modifier hotkey tokens (SPEC v0.3). */
const SPECIAL_HOTKEY_TOKENS = new Set([
  "RAlt",
  "LAlt",
  "RCtrl",
  "LCtrl",
  "RShift",
  "LShift",
]);

/**
 * Display form of a hotkey string: special single-modifier tokens render via
 * their locale key ("RAlt" → 右Alt / Right Alt / ...); combos render raw.
 * Use everywhere a hotkey is shown to the user.
 */
export function displayHotkey(hotkey: string): string {
  return SPECIAL_HOTKEY_TOKENS.has(hotkey) ? t(`hotkey.${hotkey}`) : hotkey;
}

/** Messages whose params are hotkey combos/tokens — localize special tokens. */
const HOTKEY_MSG_KEYS = new Set(["ERR_HOTKEY_PARSE", "ERR_HOTKEY_REGISTER"]);

/**
 * Render a backend-originated message of the form "KEY|p0|p1" via the
 * "msg.KEY" dictionary entry. Strings whose leading segment is not in the
 * dictionary are returned unchanged (fallback for free-form text).
 */
export function tMsg(raw: string): string {
  const parts = raw.split("|");
  const key = `msg.${parts[0]}`;
  const template = DICTS[currentLang][key] ?? FALLBACK[key];
  if (template === undefined) return raw;
  let params: string[] = parts.slice(1);
  // A detail param may itself be a nested message key (e.g. ERR_STARTUP_HOTKEY
  // carries "ERR_HOTKEY_REGISTER|Ctrl+Alt+Space") — translate it recursively.
  if (params.length > 0) {
    const nestedKey = `msg.${params[0]}`;
    if ((DICTS[currentLang][nestedKey] ?? FALLBACK[nestedKey]) !== undefined) {
      params = [tMsg(params.join("|"))];
    } else if (HOTKEY_MSG_KEYS.has(parts[0])) {
      params = params.map(displayHotkey);
    }
  }
  return substitute(template, params);
}

/**
 * Apply the current language to all static DOM nodes carrying
 * data-i18n / data-i18n-placeholder / data-i18n-title / data-i18n-aria-label
 * attributes. Called on startup and on every language change. Only touches
 * annotated nodes — never rebuilds inputs or user-entered values.
 */
export function applyDom(root: ParentNode = document): void {
  for (const node of root.querySelectorAll<HTMLElement>("[data-i18n]")) {
    const key = node.dataset.i18n;
    if (key) node.textContent = t(key);
  }
  for (const node of root.querySelectorAll<HTMLElement>("[data-i18n-placeholder]")) {
    const key = node.dataset.i18nPlaceholder;
    if (key) node.setAttribute("placeholder", t(key));
  }
  for (const node of root.querySelectorAll<HTMLElement>("[data-i18n-title]")) {
    const key = node.dataset.i18nTitle;
    if (key) node.title = t(key);
  }
  for (const node of root.querySelectorAll<HTMLElement>("[data-i18n-aria-label]")) {
    const key = node.dataset.i18nAriaLabel;
    if (key) node.setAttribute("aria-label", t(key));
  }
  document.documentElement.lang = currentLang;
}
