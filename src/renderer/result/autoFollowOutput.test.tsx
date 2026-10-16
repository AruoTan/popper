import { act, renderHook } from '@testing-library/react'
import { createRef } from 'react'
import { describe, expect, it } from 'vitest'

import { isNearScrollBottom, useAutoFollowOutput } from './autoFollowOutput'

describe('result scrollbar user control', () => {
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

  it('keeps scroll position entirely controlled by the user without moving scrollbar on streaming/rerender', () => {
    const scroller = document.createElement('div')
    Object.defineProperties(scroller, {
      scrollHeight: { configurable: true, value: 900 },
      clientHeight: { configurable: true, value: 200 },
      scrollTop: { configurable: true, writable: true, value: 350 }
    })
    const scrollRef = createRef<HTMLDivElement>()
    const observedRef = createRef<HTMLDivElement>()
    scrollRef.current = scroller
    observedRef.current = document.createElement('div')

    const { result, rerender } = renderHook(
      ({ key, enabled }) => useAutoFollowOutput(scrollRef, observedRef, key, enabled),
      { initialProps: { key: 'ai-1', enabled: true } }
    )

    // Initial position should remain intact
    expect(scroller.scrollTop).toBe(350)

    // User scrolls to 400
    scroller.scrollTop = 400
    act(() => result.current({ currentTarget: scroller } as never))
    expect(scroller.scrollTop).toBe(400)

    // Streaming updates or rerenders should NOT force scrollTop to scrollHeight
    rerender({ key: 'ai-1', enabled: true })
    expect(scroller.scrollTop).toBe(400)

    // Rerender with different keys also does not force scrollbar to jump
    rerender({ key: 'ai-2', enabled: true })
    expect(scroller.scrollTop).toBe(400)
  })
})

