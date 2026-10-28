import { describe, expect, it } from 'vitest'

import {
  advanceByUnicodeScalars,
  DEFAULT_SMOOTH_STREAM_CONFIG,
  SmoothStreamController,
  smoothStreamRate,
  smoothStreamStep
} from './streamPlayback'

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
    const config = { ...DEFAULT_SMOOTH_STREAM_CONFIG, baseCharsPerSec: 125, maxCharsPerSec: 125 }
    const first = smoothStreamStep(20, 4, 0, config)
    expect(first).toEqual({ step: 0, residual: 0.5 })
    const second = smoothStreamStep(20, 4, first.residual, config)
    expect(second).toEqual({ step: 1, residual: 0 })
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
