import { describe, expect, it, vi } from 'vitest'

import {
  flushGraphemeTail,
  MAX_GRAPHEME_CARRY_SCALARS,
  splitGraphemes,
  splitStableGraphemeTail
} from './streamPlayback'

describe('bounded grapheme tail', () => {
  it.each([
    ['combining mark', 'e', '\u0301', 'e\u0301'],
    ['ZWJ family', '👨', '\u200d👩', '👨\u200d👩'],
    ['variation selector', '❤', '\ufe0f', '❤️'],
    ['skin tone', '👍', '🏽', '👍🏽'],
    ['regional flag', '🇨', '🇳', '🇨🇳'],
    ['Hangul Jamo', 'ᄀ', 'ᅡ', '가']
  ])('keeps the %s tail bounded across deltas', (_name, first, second, combined) => {
    const held = splitStableGraphemeTail('', first)
    expect(held.carry).toBe(first)
    const extended = splitStableGraphemeTail(held.carry, second)
    expect(`${extended.released}${extended.carry}`).toBe(combined)
  })

  it('segments only a bounded suffix and bounds the carry in Unicode scalars', () => {
    const segment = vi.fn(splitGraphemes)
    const prefix = 'a'.repeat(1_000_000)
    const update = splitStableGraphemeTail('', `${prefix}😀`, segment)

    expect(segment).toHaveBeenCalledTimes(1)
    expect(Array.from(segment.mock.calls[0]![0]).length).toBeLessThanOrEqual(512)
    expect(Array.from(update.carry).length).toBeLessThanOrEqual(MAX_GRAPHEME_CARRY_SCALARS)
    expect(`${update.released}${update.carry}`).toBe(`${prefix}😀`)
  })

  it('flushes the terminal grapheme tail', () => {
    const held = splitStableGraphemeTail('', 'e')
    expect(flushGraphemeTail(held.carry)).toBe('e')
  })
})
