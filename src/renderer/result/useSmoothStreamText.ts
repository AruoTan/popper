import { useLayoutEffect, useRef, useState } from 'react'

import {
  DEFAULT_SMOOTH_STREAM_CONFIG,
  SmoothStreamController,
  type SmoothStreamConfig
} from './streamPlayback'

export type SmoothStreamFrameScheduler = (
  callback: FrameRequestCallback
) => number

export type SmoothStreamFrameCanceller = (handle: number) => void

export type SmoothStreamClock = () => number

/**
 * Display-layer adaptive typewriter. `target` stays the full logical stream
 * text from the store; returned string lags slightly so bursts paint smoothly.
 *
 * - Non-streaming / terminal: always returns `target` immediately (no lag).
 * - Streaming: rAF pump only while backlog remains; stops when caught up and
 *   restarts on the next target growth (no forever-rAF between tokens).
 * - First non-empty target: small immediate peek for TTFB, rest typewrites.
 * - useLayoutEffect seeds before paint so first-burst is not a post-paint flash.
 */
export function useSmoothStreamText(
  target: string,
  streaming: boolean,
  resetKey: string,
  config: SmoothStreamConfig = DEFAULT_SMOOTH_STREAM_CONFIG,
  requestFrame: SmoothStreamFrameScheduler = (cb) => window.requestAnimationFrame(cb),
  cancelFrame: SmoothStreamFrameCanceller = (id) => window.cancelAnimationFrame(id),
  now: SmoothStreamClock = () => performance.now()
): string {
  const controllerRef = useRef<SmoothStreamController | null>(null)
  if (controllerRef.current === null) {
    controllerRef.current = new SmoothStreamController(config)
  }
  const controller = controllerRef.current

  const targetRef = useRef(target)
  const streamingRef = useRef(streaming)
  targetRef.current = target
  streamingRef.current = streaming

  const [displayed, setDisplayed] = useState('')
  const resetKeyRef = useRef(resetKey)

  useLayoutEffect(() => {
    if (resetKeyRef.current !== resetKey) {
      resetKeyRef.current = resetKey
      controller.reset()
    }

    if (!streaming) {
      const snapped = controller.setTarget(target, false, now())
      setDisplayed((current) => (current === snapped ? current : snapped))
      return
    }

    const seeded = controller.setTarget(target, true, now())
    setDisplayed((current) => (current === seeded ? current : seeded))

    if (!controller.needsTick()) return

    let handle: number | null = null
    let cancelled = false

    const pump = (frameTime: number): void => {
      handle = null
      if (cancelled || !streamingRef.current) return

      // Pull latest target so mid-pump IPC growth is absorbed without waiting
      // for a React commit (store updates still re-run this effect too).
      controller.setTarget(targetRef.current, true, frameTime)
      if (!controller.needsTick()) return

      const advanced = controller.tick(frameTime)
      setDisplayed((current) => (current === advanced ? current : advanced))
      if (controller.needsTick()) {
        handle = requestFrame(pump)
      }
    }

    handle = requestFrame(pump)
    return () => {
      cancelled = true
      if (handle !== null) cancelFrame(handle)
    }
  }, [cancelFrame, controller, now, requestFrame, resetKey, streaming, target])

  if (!streaming) return target
  return displayed
}
