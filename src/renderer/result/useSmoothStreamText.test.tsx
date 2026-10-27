import { act, render, screen } from '@testing-library/react'
import { useState, type ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { useSmoothStreamText, type SmoothStreamFrameScheduler } from './useSmoothStreamText'

function Harness({
  target,
  streaming,
  resetKey,
  requestFrame,
  cancelFrame,
  now
}: {
  target: string
  streaming: boolean
  resetKey: string
  requestFrame: SmoothStreamFrameScheduler
  cancelFrame: (id: number) => void
  now: () => number
}): ReactElement {
  const displayed = useSmoothStreamText(
    target,
    streaming,
    resetKey,
    undefined,
    requestFrame,
    cancelFrame,
    now
  )
  return <div data-testid="out">{displayed}</div>
}

describe('useSmoothStreamText', () => {
  function BrowserHarness({ target, streaming = true, resetKey = 'request-1' }: {
    target: string; streaming?: boolean; resetKey?: string
  }): ReactElement {
    const displayed = useSmoothStreamText(target, streaming, resetKey)
    return <div data-testid="out">{displayed}</div>
  }

  function browserFrames() {
    let clock = 100, nextId = 0
    const pending = new Map<number, FrameRequestCallback>()
    vi.spyOn(performance, 'now').mockImplementation(() => clock)
    const request = vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
      pending.set(++nextId, callback)
      return nextId
    })
    const cancel = vi.spyOn(window, 'cancelAnimationFrame').mockImplementation((id) => { pending.delete(id) })
    const tick = (): void => {
      const next = pending.entries().next().value
      if (!next) throw new Error('No pending browser frame')
      pending.delete(next[0])
      clock += 16
      act(() => next[1](clock))
    }
    return { pending, request, cancel, tick }
  }

  it('keeps one default browser pump through progress and unrelated rerenders', () => {
    const frames = browserFrames()
    const target = '文'.repeat(80)
    const view = render(<BrowserHarness target={target} />)
    expect(frames.pending.size).toBe(1)
    expect(frames.request).toHaveBeenCalledTimes(1)
    const initial = screen.getByTestId('out').textContent!
    view.rerender(<BrowserHarness target={target} />)
    expect(frames.request).toHaveBeenCalledTimes(1)
    expect(frames.cancel).not.toHaveBeenCalled()

    frames.tick()
    expect(screen.getByTestId('out').textContent!.length).toBeGreaterThan(initial.length)
    expect(frames.request).toHaveBeenCalledTimes(2)
    expect(frames.cancel).not.toHaveBeenCalled()
    let ticks = 0
    while (frames.pending.size && ticks++ < 200) {
      expect(frames.pending.size).toBe(1)
      frames.tick()
    }
    expect(screen.getByTestId('out')).toHaveTextContent(target)
    expect(frames.pending.size).toBe(0)
  })

  it('replaces the default pump on a new request and cancels it on unmount', () => {
    const frames = browserFrames()
    const view = render(<BrowserHarness target={'旧'.repeat(80)} />)
    const oldId = frames.pending.keys().next().value
    view.rerender(<BrowserHarness target={'新'.repeat(80)} resetKey="request-2" />)
    expect(frames.pending.has(oldId!)).toBe(false)
    expect(frames.pending.size).toBe(1)
    expect(screen.getByTestId('out')).toHaveTextContent('新'.repeat(8))
    view.unmount()
    expect(frames.pending.size).toBe(0)
  })

  it('cancels the default pump and shows full text when streaming ends', () => {
    const frames = browserFrames()
    const target = '完成'.repeat(40)
    const view = render(<BrowserHarness target={target} />)
    view.rerender(<BrowserHarness target={target} streaming={false} />)
    expect(frames.pending.size).toBe(0)
    expect(screen.getByTestId('out')).toHaveTextContent(target)
  })

  it('returns full target immediately when not streaming', () => {
    const requestFrame = vi.fn<SmoothStreamFrameScheduler>()
    const cancelFrame = vi.fn()
    render(
      <Harness
        target="完整结果"
        streaming={false}
        resetKey="r1"
        requestFrame={requestFrame}
        cancelFrame={cancelFrame}
        now={() => 0}
      />
    )
    expect(screen.getByTestId('out')).toHaveTextContent('完整结果')
    expect(requestFrame).not.toHaveBeenCalled()
  })

  it('shows first-burst immediately then drains backlog on rAF ticks', () => {
    let clock = 0
    const frames: FrameRequestCallback[] = []
    const requestFrame: SmoothStreamFrameScheduler = (cb) => {
      frames.push(cb)
      return frames.length
    }
    const cancelFrame = vi.fn()

    const long = `你好${'字'.repeat(40)}`
    const { rerender } = render(
      <Harness
        target="你好"
        streaming
        resetKey="r1"
        requestFrame={requestFrame}
        cancelFrame={cancelFrame}
        now={() => clock}
      />
    )
    expect(screen.getByTestId('out')).toHaveTextContent('你好')

    rerender(
      <Harness
        target={long}
        streaming
        resetKey="r1"
        requestFrame={requestFrame}
        cancelFrame={cancelFrame}
        now={() => clock}
      />
    )
    // Still at first-burst seed until frames advance.
    expect(screen.getByTestId('out').textContent).toBe('你好')
    expect(frames.length).toBeGreaterThan(0)

    act(() => {
      clock += 16
      const cb = frames.shift()
      cb?.(clock)
    })
    const afterOne = screen.getByTestId('out').textContent ?? ''
    expect(afterOne.startsWith('你好')).toBe(true)
    expect(afterOne.length).toBeGreaterThan(2)
    expect(afterOne.length).toBeLessThan(long.length)

    let guard = 0
    while ((screen.getByTestId('out').textContent ?? '') !== long && guard < 200) {
      act(() => {
        clock += 16
        const cb = frames.shift()
        if (cb) cb(clock)
      })
      guard += 1
    }
    expect(screen.getByTestId('out')).toHaveTextContent(long)
  })

  it('returns the complete result immediately when streaming ends mid-drain', () => {
    let clock = 0
    const frames: FrameRequestCallback[] = []
    const requestFrame: SmoothStreamFrameScheduler = (cb) => {
      frames.push(cb)
      return frames.length
    }

    const long = `ab${'c'.repeat(40)}`
    const { rerender } = render(
      <Harness
        target={long}
        streaming
        resetKey="r1"
        requestFrame={requestFrame}
        cancelFrame={vi.fn()}
        now={() => clock}
      />
    )
    expect((screen.getByTestId('out').textContent ?? '').length).toBeLessThan(long.length)

    rerender(
      <Harness
        target={long}
        streaming={false}
        resetKey="r1"
        requestFrame={requestFrame}
        cancelFrame={vi.fn()}
        now={() => clock}
      />
    )
    expect(screen.getByTestId('out')).toHaveTextContent(long)
  })

  it('resets displayed text when resetKey changes', () => {
    let clock = 0
    const frames: FrameRequestCallback[] = []
    const requestFrame: SmoothStreamFrameScheduler = (cb) => {
      frames.push(cb)
      return frames.length
    }

    function Driver(): ReactElement {
      const [key, setKey] = useState('req-a')
      const [target, setTarget] = useState('第一段内容较多一些文字')
      return (
        <>
          <Harness
            target={target}
            streaming
            resetKey={key}
            requestFrame={requestFrame}
            cancelFrame={vi.fn()}
            now={() => clock}
          />
          <button
            type="button"
            onClick={() => {
              setKey('req-b')
              setTarget('第二')
            }}
          >
            next
          </button>
        </>
      )
    }

    render(<Driver />)
    expect(screen.getByTestId('out').textContent?.length).toBeGreaterThan(0)

    act(() => {
      screen.getByRole('button', { name: 'next' }).click()
    })
    // New request first-burst for short "第二"
    expect(screen.getByTestId('out')).toHaveTextContent('第二')
  })

  it('stops the pump when backlog is empty (no forever rAF)', () => {
    let clock = 0
    const frames: FrameRequestCallback[] = []
    const requestFrame: SmoothStreamFrameScheduler = (cb) => {
      frames.push(cb)
      return frames.length
    }

    render(
      <Harness
        target="短"
        streaming
        resetKey="r1"
        requestFrame={requestFrame}
        cancelFrame={vi.fn()}
        now={() => clock}
      />
    )
    // Fully shown via first-burst — no backlog to animate.
    expect(screen.getByTestId('out')).toHaveTextContent('短')
    expect(frames).toHaveLength(0)
  })
})
