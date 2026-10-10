export const APP_NAME = "Popper";
export const APP_ID = "com.local.popper";

export const SETTINGS_VERSION = 16 as const;
export const AI_TEXT_LIMIT = 20_000;
export const AI_PROMPT_LIMIT = 50_000;
export const AI_OUTPUT_LIMIT = 1_000_000;
export const MAX_ENABLED_ACTIONS = 8;
export const MAX_ACTIONS = 50;
export const MAX_PROVIDERS = 20;
export const MAX_PROVIDER_MODELS = 200;
export const RESULT_FONT_SIZE_MIN = 12;
export const RESULT_FONT_SIZE_MAX = 24;
export const DEFAULT_RESULT_FONT_SIZE = 14;

export const TEXT_PLACEHOLDER = "{{text}}";
export const OUTPUT_LANGUAGE_PLACEHOLDER = "{{language}}";
export const TARGET_LANGUAGE_PLACEHOLDER = "{{target_language}}";
export const DEFAULT_OPENAI_BASE_URL = "https://api.openai.com/v1";
export const DEFAULT_PROVIDER_ID = "openai-compatible";
export const DEFAULT_PROVIDER_NAME = "OpenAI Compatible";
/** @deprecated Models now live under providers. */
export const DEFAULT_MODEL = "";

export const DEFAULT_LOCALE = "zh-CN" as const;
export const DEFAULT_TRANSLATION_PAIR = {
  dictionaryEnabled: true,
  primaryLanguage: "zh-CN",
  alternateLanguage: "en-US",
} as const;

export const DEFAULT_RESULT_SETTINGS = {
  followCursor: true,
  rememberSize: true,
  defaultPinned: false,
  dismissMode: "blur",
  dismissDelayMs: 450,
  opacity: 1,
  fontSize: DEFAULT_RESULT_FONT_SIZE,
  lastSize: null,
} as const;

export const DEFAULT_FILTER_SETTINGS = {
  mode: "default",
  applications: [],
} as const;

export const TOOLBAR_SCREEN_GAP = 8;
export const TOOLBAR_SCREEN_MARGIN = 8;
