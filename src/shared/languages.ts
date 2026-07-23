/** UI / AI output locale (settings interface + {{language}} for summary/explain). */
export const UI_LOCALES = ['zh-CN', 'en-US'] as const
export type UiLocale = (typeof UI_LOCALES)[number]

/** Target languages available for the translate action and result-box switcher. */
export const TRANSLATION_LANGUAGES = [
  'zh-CN',
  'en-US',
  'ja-JP',
  'ko-KR',
  'ru-RU',
  'de-DE',
  'fr-FR'
] as const
export type TranslationLanguage = (typeof TRANSLATION_LANGUAGES)[number]

export const TRANSLATION_LANGUAGE_NAMES: Readonly<Record<TranslationLanguage, string>> = {
  'zh-CN': '简体中文',
  'en-US': 'English',
  'ja-JP': '日本語',
  'ko-KR': '한국어',
  'ru-RU': 'Русский',
  'de-DE': 'Deutsch',
  'fr-FR': 'Français'
}

/** Short codes shown in the result-window translation route. */
export const TRANSLATION_LANGUAGE_CODES: Readonly<Record<TranslationLanguage, string>> = {
  'zh-CN': 'CN',
  'en-US': 'EN',
  'ja-JP': 'JA',
  'ko-KR': 'KO',
  'ru-RU': 'RU',
  'de-DE': 'DE',
  'fr-FR': 'FR'
}

/** English names used inside the translation prompt's {{target_language}} slot. */
export const TRANSLATION_TARGET_ENGLISH_NAMES: Readonly<Record<TranslationLanguage, string>> = {
  'zh-CN': 'Chinese (Simplified)',
  'en-US': 'English',
  'ja-JP': 'Japanese',
  'ko-KR': 'Korean',
  'ru-RU': 'Russian',
  'de-DE': 'German',
  'fr-FR': 'French'
}

export function isTranslationLanguage(value: string): value is TranslationLanguage {
  return (TRANSLATION_LANGUAGES as readonly string[]).includes(value)
}

/**
 * Detect the likely source language of selected text.
 * Chinese (Han) wins over shared CJK punctuation; Japanese needs kana;
 * Korean hangul; Cyrillic → Russian; otherwise Latin/other → English.
 */
export function detectTranslationLanguage(text: string): TranslationLanguage {
  const hasHan = /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/u.test(text)
  const hasKana = /[\u3040-\u309f\u30a0-\u30ff]/u.test(text)
  if (hasKana) return 'ja-JP'
  if (/[\uac00-\ud7af\u1100-\u11ff]/u.test(text)) return 'ko-KR'
  if (hasHan) return 'zh-CN'
  if (/[\u0400-\u04ff]/u.test(text)) return 'ru-RU'
  return 'en-US'
}

export interface TranslationPair {
  primaryLanguage: TranslationLanguage
  alternateLanguage: TranslationLanguage
}

/**
 * Resolve default translation target:
 * - If source matches the configured pair, flip to the other side.
 * - Otherwise: Chinese → English, any other language → Chinese.
 */
export function defaultTranslationTarget(
  source: TranslationLanguage,
  pair: TranslationPair
): TranslationLanguage {
  if (source === pair.primaryLanguage) return pair.alternateLanguage
  if (source === pair.alternateLanguage) return pair.primaryLanguage
  return source === 'zh-CN' ? 'en-US' : 'zh-CN'
}

export function translationTargetEnglishName(locale: TranslationLanguage): string {
  return TRANSLATION_TARGET_ENGLISH_NAMES[locale]
}
