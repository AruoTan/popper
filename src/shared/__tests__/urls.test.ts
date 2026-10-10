import { describe, expect, it } from 'vitest'

import {
  buildOpenAiEndpoint,
  isSafeExternalUrl,
  validateOpenAiBaseUrl,
} from '..'

describe('URL helpers', () => {
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

})
