import { describe, expect, it } from 'vitest'

import {
  DEFAULT_SEARCH_TEMPLATE,
  buildOpenAiEndpoint,
  buildSearchUrl,
  isSafeExternalUrl,
  resolveSearchTarget,
  validateOpenAiBaseUrl,
  validateSearchTemplate
} from '..'

describe('URL helpers', () => {
  it('encodes selection text in the search template', () => {
    expect(buildSearchUrl(DEFAULT_SEARCH_TEMPLATE, 'macOS 划词 & AI')).toBe(
      'https://www.google.com/search?q=macOS%20%E5%88%92%E8%AF%8D%20%26%20AI'
    )
  })

  it('rejects missing placeholders and unsafe protocols', () => {
    expect(validateSearchTemplate('https://google.com/search?q=fixed').valid).toBe(false)
    expect(
      validateSearchTemplate('https://google.com/search?q={{text}}&again={{text}}').valid
    ).toBe(false)
    expect(validateSearchTemplate('javascript:alert({{text}})').valid).toBe(false)
  })

  it('allows only HTTP(S) external links without credentials', () => {
    expect(isSafeExternalUrl('https://example.com/page')).toBe(true)
    expect(isSafeExternalUrl('file:///tmp/private')).toBe(false)
    expect(isSafeExternalUrl('https://user:pass@example.com')).toBe(false)
  })

  it('normalizes OpenAI-compatible endpoint paths', () => {
    expect(buildOpenAiEndpoint('http://localhost:11434/v1///', 'chat/completions')).toBe(
      'http://localhost:11434/v1/chat/completions'
    )
  })

  it('accepts HTTP and HTTPS model providers while still rejecting unsafe URL shapes', () => {
    expect(validateOpenAiBaseUrl('https://api.example.com/v1').valid).toBe(true)
    expect(validateOpenAiBaseUrl('http://localhost:11434/v1').valid).toBe(true)
    expect(validateOpenAiBaseUrl('http://127.0.0.1:11434/v1').valid).toBe(true)
    expect(validateOpenAiBaseUrl('http://api.example.com/v1').valid).toBe(true)
    expect(validateOpenAiBaseUrl('ftp://api.example.com/v1').valid).toBe(false)
    expect(validateOpenAiBaseUrl('https://user:pass@api.example.com/v1').valid).toBe(false)
  })

  it('opens explicit URLs, domains and valid IP addresses directly', () => {
    expect(resolveSearchTarget('https://example.com/docs?q=1')).toBe(
      'https://example.com/docs?q=1'
    )
    expect(resolveSearchTarget('example.com/docs')).toBe('https://example.com/docs')
    expect(resolveSearchTarget('192.168.1.20:8080/status')).toBe(
      'https://192.168.1.20:8080/status'
    )
    expect(resolveSearchTarget('[2001:db8::1]/status')).toBe(
      'https://[2001:db8::1]/status'
    )
    expect(resolveSearchTarget('2001:db8::1')).toBe('https://[2001:db8::1]/')
  })

  it('searches ordinary text and malformed or unsafe address-like text', () => {
    expect(resolveSearchTarget('Tauri 划词助手')).toBe(
      'https://www.google.com/search?q=Tauri%20%E5%88%92%E8%AF%8D%E5%8A%A9%E6%89%8B'
    )
    expect(resolveSearchTarget('999.168.1.20')).toBe(
      'https://www.google.com/search?q=999.168.1.20'
    )
    expect(resolveSearchTarget('javascript:alert(1)')).toBe(
      'https://www.google.com/search?q=javascript%3Aalert(1)'
    )
  })
})
