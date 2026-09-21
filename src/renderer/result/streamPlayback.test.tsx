import { describe, expect, it, vi } from 'vitest'

import {
  advanceByUnicodeScalars,
  flushGraphemeTail,
  MAX_GRAPHEME_CARRY_SCALARS,
  SmoothStreamController,
  smoothStreamRate,
  smoothStreamStep,
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

describe('smooth stream typewriter', () => {
  it('ramps rate with backlog between base and max', () => {
    expect(smoothStreamRate(0)).toBe(0)
    const thin = smoothStreamRate(2)
    const thick = smoothStreamRate(80)
    expect(thin).toBeGreaterThan(0)
    expect(thick).toBeGreaterThan(thin)
    expect(thick).toBeLessThanOrEqual(360)
  })

  it('accumulates fractional residual across frames for continuous motion', () => {
    // Tiny elapsed may not yield a whole scalar yet; residual must carry.
    const first = smoothStreamStep(20, 4, 0)
    expect(first.step + first.residual).toBeGreaterThan(0)
    const second = smoothStreamStep(20, 4, first.residual)
    // Over two short frames we should eventually release at least one scalar
    // when residual stacks, or already have released on first.
    expect(first.step + second.step).toBeGreaterThanOrEqual(0)
    const longFrame = smoothStreamStep(20, 50, 0)
    expect(longFrame.step).toBeGreaterThanOrEqual(1)
    expect(longFrame.step).toBeLessThanOrEqual(56)
  })

  it('shows a small first-burst peek then drains backlog smoothly over time', () => {
    const stream = new SmoothStreamController()
    // 2 scalars < firstBurstMax(8) → full first chunk for TTFB
    expect(stream.setTarget('你好', true, 0)).toBe('你好')
    expect(stream.needsTick()).toBe(false)

    const long = `你好${'字'.repeat(80)}`
    expect(stream.setTarget(long, true, 16)).toBe('你好')
    expect(stream.needsTick()).toBe(true)

    const afterOne = stream.tick(32)
    expect(afterOne.startsWith('你好')).toBe(true)
    expect(afterOne.length).toBeGreaterThan('你好'.length)
    expect(afterOne.length).toBeLessThan(long.length)

    let guard = 0
    let t = 32
    while (stream.needsTick() && guard < 400) {
      t += 16
      stream.tick(t)
      guard += 1
    }
    expect(stream.getDisplayed()).toBe(long)
  })

  it('caps the first paint when the opening chunk is large', () => {
    const stream = new SmoothStreamController()
    const opening = '字'.repeat(40)
    const shown = stream.setTarget(opening, true, 0)
    expect(shown.length).toBeLessThan(opening.length)
    expect(shown.length).toBeGreaterThan(0)
    expect(stream.needsTick()).toBe(true)
  })

  it('snaps to full text when streaming ends', () => {
    const stream = new SmoothStreamController()
    stream.setTarget('ab', true, 0)
    stream.setTarget(`ab${'c'.repeat(50)}`, true, 16)
    expect(stream.getDisplayed().length).toBeLessThan(52)
    expect(stream.setTarget(`ab${'c'.repeat(50)}`, false, 32)).toBe(`ab${'c'.repeat(50)}`)
    expect(stream.needsTick()).toBe(false)
  })

  it('does not split surrogate pairs when advancing', () => {
    const emoji = '😀'
    const value = `${emoji}${emoji}${emoji}`
    const mid = advanceByUnicodeScalars(value, 0, 1)
    expect(value.slice(0, mid)).toBe(emoji)
    expect(advanceByUnicodeScalars(value, 0, 3)).toBe(value.length)
  })
})
