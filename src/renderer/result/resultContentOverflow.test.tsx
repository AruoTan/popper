import { act, renderHook } from '@testing-library/react'
import { createRef } from 'react'

import { hasVerticalOverflow, useResultContentOverflow } from './resultContentOverflow'

let resizeCallback: ResizeObserverCallback | null = null

function setGeometry(element: HTMLElement, clientHeight: number, scrollHeight: number): void {
  Object.defineProperties(element, {
    clientHeight: { configurable: true, value: clientHeight },
    scrollHeight: { configurable: true, value: scrollHeight },
    scrollTop: { configurable: true, writable: true, value: 0 }
  })
}

beforeEach(() => {
  resizeCallback = null
  Object.defineProperty(window, 'ResizeObserver', {
    configurable: true,
    value: class ResizeObserverMock {
      constructor(callback: ResizeObserverCallback) {
        resizeCallback = callback
      }

      observe(): void {}
      disconnect(): void {}
    }
  })
})

afterEach(() => {
  Reflect.deleteProperty(window, 'ResizeObserver')
})

describe('result content overflow', () => {
  it('does not enable scrolling while waiting for the first model content', () => {
    const scroller = document.createElement('div')
    const inner = document.createElement('div')
    setGeometry(scroller, 300, 900)
    const scrollRef = createRef<HTMLDivElement>()
    const observedRef = createRef<HTMLDivElement>()
    scrollRef.current = scroller
    observedRef.current = inner

    const { result } = renderHook(() => (
      useResultContentOverflow(scrollRef, observedRef, false)
    ))

    expect(result.current).toEqual({ isOverflowing: false, trackMarginPx: 0 })
  })

  it('keeps scrolling disabled for short content', () => {
    const scroller = document.createElement('div')
    const inner = document.createElement('div')
    setGeometry(scroller, 300, 300)
    expect(hasVerticalOverflow(scroller)).toBe(false)
    const scrollRef = createRef<HTMLDivElement>()
    const observedRef = createRef<HTMLDivElement>()
    scrollRef.current = scroller
    observedRef.current = inner

    const { result } = renderHook(() => (
      useResultContentOverflow(scrollRef, observedRef, true)
    ))

    expect(result.current).toEqual({ isOverflowing: false, trackMarginPx: 0 })
  })

  it('enables native scrolling for overflow without limiting its track', async () => {
    const scroller = document.createElement('div')
    const inner = document.createElement('div')
    setGeometry(scroller, 320, 321)
    expect(hasVerticalOverflow(scroller)).toBe(false)
    const scrollRef = createRef<HTMLDivElement>()
    const observedRef = createRef<HTMLDivElement>()
    scrollRef.current = scroller
    observedRef.current = inner

    const { result } = renderHook(() => (
      useResultContentOverflow(scrollRef, observedRef, true)
    ))
    expect(result.current.isOverflowing).toBe(false)

    setGeometry(scroller, 320, 700)
    await act(async () => {
      resizeCallback?.([], {} as ResizeObserver)
      await new Promise((resolve) => window.requestAnimationFrame(resolve))
    })

    expect(result.current).toEqual({ isOverflowing: true, trackMarginPx: 0 })
  })

  it('keeps trackMarginPx at 0 regardless of viewport height to give user full control of scrollbar', () => {
    const scroller = document.createElement('div')
    const inner = document.createElement('div')
    setGeometry(scroller, 485, 900)
    const scrollRef = createRef<HTMLDivElement>()
    const observedRef = createRef<HTMLDivElement>()
    scrollRef.current = scroller
    observedRef.current = inner

    const { result } = renderHook(() => (
      useResultContentOverflow(scrollRef, observedRef, true)
    ))

    expect(result.current.trackMarginPx).toBe(0)
    expect(result.current.isOverflowing).toBe(true)
  })
})

