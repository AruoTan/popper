import { describe, expect, it } from 'vitest'

import {
  TRANSLATION_LANGUAGE_CODES,
  TRANSLATION_LANGUAGES,
  defaultTranslationTarget,
  detectTranslationLanguage,
  translationTargetEnglishName
} from '../languages'

describe('translation languages', () => {
  it('exposes JA/KO/RU/DE/FR alongside zh/en with short result codes', () => {
    expect(TRANSLATION_LANGUAGES).toEqual([
      'zh-CN',
      'en-US',
      'ja-JP',
      'ko-KR',
      'ru-RU',
      'de-DE',
      'fr-FR'
    ])
    expect(TRANSLATION_LANGUAGE_CODES['ja-JP']).toBe('JA')
    expect(TRANSLATION_LANGUAGE_CODES['ko-KR']).toBe('KO')
    expect(TRANSLATION_LANGUAGE_CODES['ru-RU']).toBe('RU')
    expect(TRANSLATION_LANGUAGE_CODES['de-DE']).toBe('DE')
    expect(TRANSLATION_LANGUAGE_CODES['fr-FR']).toBe('FR')
  })

  it('detects CJK / Hangul / Cyrillic source languages', () => {
    expect(detectTranslationLanguage('这是中文')).toBe('zh-CN')
    expect(detectTranslationLanguage('これは日本語')).toBe('ja-JP')
    expect(detectTranslationLanguage('안녕하세요')).toBe('ko-KR')
    expect(detectTranslationLanguage('Привет')).toBe('ru-RU')
    expect(detectTranslationLanguage('Hello world')).toBe('en-US')
  })

  it('defaults Chinese↔English and other languages to Chinese outside the pair', () => {
    const pair = { primaryLanguage: 'zh-CN' as const, alternateLanguage: 'en-US' as const }
    expect(defaultTranslationTarget('zh-CN', pair)).toBe('en-US')
    expect(defaultTranslationTarget('en-US', pair)).toBe('zh-CN')
    expect(defaultTranslationTarget('ja-JP', pair)).toBe('zh-CN')
    expect(defaultTranslationTarget('fr-FR', pair)).toBe('zh-CN')
  })

  it('maps prompt slot names in English', () => {
    expect(translationTargetEnglishName('de-DE')).toBe('German')
    expect(translationTargetEnglishName('zh-CN')).toBe('Chinese (Simplified)')
  })
})
