import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject
} from 'react'

const OVERFLOW_TOLERANCE_PX = 1

export interface ResultContentOverflowState {
  isOverflowing: boolean
  trackMarginPx: number
}

const INITIAL_STATE: ResultContentOverflowState = {
  isOverflowing: false,
  trackMarginPx: 0
}

export function hasVerticalOverflow(element: HTMLElement): boolean {
  return element.scrollHeight > element.clientHeight + OVERFLOW_TOLERANCE_PX
}

/**
 * 测量内容是否溢出，以在需要时展示原生滚动条。
 * 滚动条轨道铺满整个高度（trackMarginPx 保持为 0），不将滑动条限制或固定在中间某一处。
 * 滑动条位置完全由用户掌控，不强制重置 scrollTop。
 */
export function useResultContentOverflow(
  scrollRef: RefObject<HTMLDivElement | null>,
  observedRef: RefObject<HTMLDivElement | null>,
  enabled: boolean
): ResultContentOverflowState {
  const [state, setState] = useState<ResultContentOverflowState>(INITIAL_STATE)
  const frameRef = useRef<number | null>(null)

  const measure = useCallback((): void => {
    const element = scrollRef.current
    const isOverflowing = Boolean(element && enabled && hasVerticalOverflow(element))
    const trackMarginPx = 0

    setState((current) => (
      current.isOverflowing === isOverflowing && current.trackMarginPx === trackMarginPx
        ? current
        : { isOverflowing, trackMarginPx }
    ))
  }, [enabled, scrollRef])


  const scheduleMeasure = useCallback((): void => {
    if (frameRef.current !== null) return
    frameRef.current = window.requestAnimationFrame(() => {
      frameRef.current = null
      measure()
    })
  }, [measure])

  useLayoutEffect(() => {
    measure()

    const scrollElement = scrollRef.current
    const observedElement = observedRef.current
    const observer = typeof ResizeObserver === 'function'
      ? new ResizeObserver(scheduleMeasure)
      : null

    if (scrollElement) observer?.observe(scrollElement)
    if (observedElement && observedElement !== scrollElement) observer?.observe(observedElement)
    window.addEventListener('resize', scheduleMeasure)

    return () => {
      observer?.disconnect()
      window.removeEventListener('resize', scheduleMeasure)
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current)
        frameRef.current = null
      }
    }
  }, [measure, observedRef, scheduleMeasure, scrollRef])

  return state
}
