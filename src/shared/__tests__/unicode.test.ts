import { describe, expect, it } from 'vitest'

import { countUnicodeScalars, hasAtMostUnicodeScalars } from '../unicode'

describe('Unicode scalar helpers', () => {
  it('counts BMP and astral values as Unicode scalars', () => {
    expect(countUnicodeScalars('A😀𠮷')).toBe(3)
    expect(countUnicodeScalars('e\u0301')).toBe(2)
  })

  it('checks a scalar limit without treating surrogate pairs as two values', () => {
    expect(hasAtMostUnicodeScalars('😀'.repeat(4), 4)).toBe(true)
    expect(hasAtMostUnicodeScalars('😀'.repeat(5), 4)).toBe(false)
  })

  it('rejects negative limits and accepts the empty string at zero', () => {
    expect(hasAtMostUnicodeScalars('', 0)).toBe(true)
    expect(hasAtMostUnicodeScalars('A', 0)).toBe(false)
    expect(() => hasAtMostUnicodeScalars('', -1)).toThrow(RangeError)
  })
})
