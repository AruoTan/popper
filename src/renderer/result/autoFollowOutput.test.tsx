import { act, renderHook } from '@testing-library/react'
import { createRef } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { isNearScrollBottom, useAutoFollowOutput } from './autoFollowOutput'

let resizeCallback: ResizeObserverCallback | null = null
const disconnect = vi.fn()

beforeEach(() => {
  vi.useFakeTimers()
  resizeCallback = null
  disconnect.mockReset()
  Object.defineProperty(window, 'ResizeObserver', {
    configurable: true,
    value: class {
      constructor(callback: ResizeObserverCallback) {
        resizeCallback = callback
      }
      observe(): void {}
      disconnect = disconnect
    }
  })
})

afterEach(() => {
  vi.useRealTimers()
  Reflect.deleteProperty(window, 'ResizeObserver')
})

describe('result auto-follow scrolling', () => {
  it('opens dictionary results at the top, preserves reading position, and resets for a new word', async () => {
    const scroller = document.createElement('div')
    Object.defineProperties(scroller, {
      scrollHeight: { configurable: true, value: 900 },
      clientHeight: { configurable: true, value: 200 },
      scrollTop: { configurable: true, writable: true, value: 700 }
    })
    const scrollRef = createRef<HTMLDivElement>()
    const observedRef = createRef<HTMLDivElement>()
    scrollRef.current = scroller
    observedRef.current = document.createElement('div')
    const { result, rerender } = renderHook(
      ({ key, enabled }) => useAutoFollowOutput(scrollRef, observedRef, key, enabled),
      { initialProps: { key: 'ai-1', enabled: true } }
    )
    // Switching before a queued AI scroll runs must cancel that scroll.
    rerender({ key: 'dictionary-1', enabled: false })
    await act(async () => vi.advanceTimersByTimeAsync(16))
    expect(scroller.scrollTop).toBe(0)

    scroller.scrollTop = 650
    act(() => result.current({ currentTarget: scroller } as never))
    rerender({ key: 'dictionary-1', enabled: false })
    await act(async () => vi.advanceTimersByTimeAsync(16))
    expect(scroller.scrollTop).toBe(650)

    rerender({ key: 'dictionary-2', enabled: false })
    expect(scroller.scrollTop).toBe(0)

    rerender({ key: 'ai-2', enabled: true })
    await act(async () => vi.advanceTimersByTimeAsync(16))
    expect(scroller.scrollTop).toBe(900)
  })

  it('recognizes whether the user is near the bottom', () => {
    const element = document.createElement('div')
    Object.defineProperties(element, {
      scrollHeight: { configurable: true, value: 500 },
      clientHeight: { configurable: true, value: 200 },
      scrollTop: { configurable: true, writable: true, value: 250 }
    })
    expect(isNearScrollBottom(element)).toBe(true)
    element.scrollTop = 100
    expect(isNearScrollBottom(element)).toBe(false)
  })

  it('coalesces resize updates and stops following after the user scrolls away', async () => {
    const scroller = document.createElement('div')
    const observed = document.createElement('div')
    Object.defineProperties(scroller, {
      scrollHeight: { configurable: true, value: 500 },
      clientHeight: { configurable: true, value: 200 },
      scrollTop: { configurable: true, writable: true, value: 0 }
    })
    const scrollRef = createRef<HTMLDivElement>()
    const observedRef = createRef<HTMLDivElement>()
    scrollRef.current = scroller
    observedRef.current = observed
    const { result } = renderHook(() => useAutoFollowOutput(scrollRef, observedRef, 'request-1'))

    await act(async () => vi.advanceTimersByTimeAsync(16))
    expect(scroller.scrollTop).toBe(500)

    scroller.scrollTop = 100
    act(() => result.current({ currentTarget: scroller } as never))
    act(() => {
      resizeCallback?.([], {} as ResizeObserver)
      resizeCallback?.([], {} as ResizeObserver)
    })
    await act(async () => vi.advanceTimersByTimeAsync(16))
    expect(scroller.scrollTop).toBe(100)
  })
})
