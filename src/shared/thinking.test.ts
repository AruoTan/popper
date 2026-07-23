// src/shared/thinking.test.ts
import { describe, expect, it } from 'vitest'
import {
  clampThinkingMode,
  effectiveThinkingLevels,
  inferThinkingLevels,
  modelSupportsThinkingOff,
  THINKING_LEVELS
} from './thinking'

describe('inferThinkingLevels', () => {
  it('returns empty for ordinary chat models', () => {
    expect(inferThinkingLevels('gpt-4o-mini')).toEqual([])
    expect(inferThinkingLevels('qwen-plus')).toEqual([])
  })

  it('detects OpenAI o-series and gpt-5 reasoning models', () => {
    expect(inferThinkingLevels('o3-mini')).toEqual(['low', 'medium', 'high'])
    expect(inferThinkingLevels('o1')).toEqual(['low', 'medium', 'high'])
    expect(inferThinkingLevels('gpt-5')).toContain('medium')
  })

  it('detects deepseek-r1 / qwen3 thinking / claude extended patterns', () => {
    expect(inferThinkingLevels('deepseek-r1')).toEqual(['low', 'medium', 'high'])
    expect(inferThinkingLevels('qwen3-235b-a22b')).toEqual(['low', 'medium', 'high'])
    expect(inferThinkingLevels('claude-opus-4-thinking')).toContain('high')
  })
})

describe('clampThinkingMode', () => {
  it('keeps off always', () => {
    expect(clampThinkingMode('off', ['low', 'high'])).toBe('off')
    expect(clampThinkingMode('off', [])).toBe('off')
  })

  it('falls back to off when model has no levels or mode unsupported', () => {
    expect(clampThinkingMode('high', [])).toBe('off')
    expect(clampThinkingMode('xhigh', ['low', 'medium'])).toBe('off')
  })

  it('keeps supported mode', () => {
    expect(clampThinkingMode('medium', ['low', 'medium', 'high'])).toBe('medium')
  })
})

describe('effectiveThinkingLevels', () => {
  it('prefers stored metadata over inference', () => {
    expect(effectiveThinkingLevels('gpt-4o', ['low', 'high'])).toEqual(['low', 'high'])
    expect(effectiveThinkingLevels('deepseek-r1', ['medium'])).toEqual(['medium'])
  })

  it('falls back to model-id heuristics when metadata is empty', () => {
    expect(effectiveThinkingLevels('deepseek-r1', [])).toEqual(['low', 'medium', 'high'])
    expect(effectiveThinkingLevels('gpt-4o-mini', [])).toEqual([])
  })
})

describe('modelSupportsThinkingOff', () => {
  it('is true whenever thinking levels exist', () => {
    expect(modelSupportsThinkingOff(['low', 'medium'])).toBe(true)
    expect(modelSupportsThinkingOff([])).toBe(false)
  })
})
