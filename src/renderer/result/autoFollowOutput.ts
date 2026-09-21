import { useCallback, useEffect, useRef, type RefObject, type UIEvent } from 'react'

const BOTTOM_THRESHOLD_PX = 64

export function isNearScrollBottom(element: HTMLElement): boolean {
  return element.scrollHeight - element.scrollTop - element.clientHeight < BOTTOM_THRESHOLD_PX
}

export function useAutoFollowOutput(
  scrollRef: RefObject<HTMLDivElement | null>,
  observedRef: RefObject<HTMLDivElement | null>,
  resetKey: string | null,
  enabled = true
): (event: UIEvent<HTMLDivElement>) => void {
  const followingRef = useRef(true)
  const frameRef = useRef<number | null>(null)

  const scrollToBottom = useCallback((): void => {
    if (!enabled) return
    if (frameRef.current !== null) return
    frameRef.current = window.requestAnimationFrame(() => {
      frameRef.current = null
      const element = scrollRef.current
      if (element && followingRef.current) element.scrollTop = element.scrollHeight
    })
  }, [scrollRef, enabled])

  useEffect(() => {
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current)
      frameRef.current = null
    }
    followingRef.current = enabled
    if (enabled) scrollToBottom()
    else if (scrollRef.current) scrollRef.current.scrollTop = 0
  }, [resetKey, scrollToBottom, scrollRef, enabled])

  useEffect(() => {
    const observed = observedRef.current
    if (!enabled || !observed || typeof ResizeObserver !== 'function') return
    const observer = new ResizeObserver(scrollToBottom)
    observer.observe(observed)
    return () => observer.disconnect()
  }, [observedRef, scrollToBottom, enabled])

  useEffect(() => () => {
    if (frameRef.current !== null) window.cancelAnimationFrame(frameRef.current)
  }, [])

  return useCallback((event: UIEvent<HTMLDivElement>): void => {
    followingRef.current = isNearScrollBottom(event.currentTarget)
  }, [])
}
