import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { ActionStreamEvent, ResultSessionSnapshot } from '../../shared'

let receiveEvent: ((event: ActionStreamEvent) => void) | null = null
let frameCallbacks = new Map<number, FrameRequestCallback>()
let nextFrameId = 1

const eventBase = {
  sessionId: 'session-1',
  sessionGeneration: 9,
  requestId: 'request-1',
  requestGeneration: 1,
  actionId: 'summary'
} as const

type EventOverrides = Partial<Pick<
  ActionStreamEvent,
  | 'sessionId'
  | 'sessionGeneration'
  | 'requestId'
  | 'requestGeneration'
  | 'actionId'
>>

const started = (sequence = 1, overrides: EventOverrides = {}): ActionStreamEvent => ({
  ...eventBase,
  ...overrides,
  type: 'started',
  sequence
})

const delta = (
  sequence: number,
  value: string,
  overrides: EventOverrides = {}
): ActionStreamEvent => ({
  ...eventBase,
  ...overrides,
  type: 'delta',
  sequence,
  delta: value
})

function snapshot(
  overrides: Partial<ResultSessionSnapshot> = {}
): ResultSessionSnapshot {
  return {
    sessionId: 'session-1',
    sessionGeneration: 9,
    requestId: 'request-1',
    requestGeneration: 1,
    actionId: 'summary',
    providerId: 'openai-compatible',
    modelId: 'model-1',
    selection: {
      selectionId: 'selection-1',
      text: 'hello',
      sourceApp: { name: 'Test', bundleId: null },
      anchor: { kind: 'cursor', x: 10, y: 20 },
      direction: 'unknown',
      isFullscreen: false
    },
    status: 'streaming',
    content: '',
    contentScalarCount: 0,
    lastContentSequence: 0,
    lastSequence: 0,
    handshakeGeneration: 4,
    errorMessage: '',
    retryable: false,
    pinned: false,
    ...overrides
  }
}

function emit(event: ActionStreamEvent): void {
  if (!receiveEvent) throw new Error('action event bridge is not listening')
  receiveEvent(event)
}

function runNextFrame(now = 16): void {
  const [id, callback] = frameCallbacks.entries().next().value ?? []
  if (id === undefined || !callback) throw new Error('No frame was scheduled')
  frameCallbacks.delete(id)
  callback(now)
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.resetModules()
  receiveEvent = null
  frameCallbacks = new Map()
  nextFrameId = 1
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
    const id = nextFrameId++
    frameCallbacks.set(id, callback)
    return id
  })
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation((id) => {
    frameCallbacks.delete(id)
  })
  Object.defineProperty(window, 'textLens', {
    configurable: true,
    value: {
      onActionEvent: vi.fn((listener: (event: ActionStreamEvent) => void) => {
        receiveEvent = listener
        return () => {
          receiveEvent = null
        }
      })
    }
  })
})

afterEach(() => {
  vi.restoreAllMocks()
  vi.useRealTimers()
  Reflect.deleteProperty(window, 'textLens')
})

describe('action event store publication pacing', () => {
  it('publishes first content synchronously and coalesces later deltas by frame', async () => {
    const store = await import('./actionEventStore')
    const stop = store.startActionEventStore()
    const listener = vi.fn()
    const unsubscribe = store.subscribeToActionEvents(listener)

    emit(started())
    expect(listener).toHaveBeenCalledTimes(1)
    emit(delta(2, 'A'))
    expect(listener).toHaveBeenCalledTimes(2)

    for (let index = 0; index < 10_000; index += 1) {
      emit(delta(index + 3, 'x'))
    }
    expect(listener).toHaveBeenCalledTimes(2)
    expect(frameCallbacks.size).toBe(1)

    runNextFrame()
    expect(listener).toHaveBeenCalledTimes(3)
    expect(store.getActionEventSnapshot().content).toBe(`A${'x'.repeat(9_999)}`)

    runNextFrame(32)
    expect(listener).toHaveBeenCalledTimes(4)
    expect(store.getActionEventSnapshot().content).toBe(`A${'x'.repeat(10_000)}`)

    unsubscribe()
    stop()
  })

  it('force-flushes queued content and publishes a valid terminal once', async () => {
    const store = await import('./actionEventStore')
    const stop = store.startActionEventStore()
    const listener = vi.fn()
    store.subscribeToActionEvents(listener)

    emit(started())
    emit(delta(2, 'A'))
    emit(delta(3, 'B'))
    emit({
      ...eventBase,
      type: 'completed',
      sequence: 4,
      lastContentSequence: 3,
      contentScalarCount: 2
    })

    expect(store.getActionEventSnapshot()).toMatchObject({
      status: 'completed',
      content: 'AB',
      contentScalarCount: 2
    })
    expect(listener).toHaveBeenCalledTimes(3)
    expect(frameCallbacks.size).toBe(0)
    expect(vi.getTimerCount()).toBe(0)
    await vi.runAllTimersAsync()
    expect(listener).toHaveBeenCalledTimes(3)
    stop()
  })

  it('uses one shared latch when the timeout wins', async () => {
    const store = await import('./actionEventStore')
    const stop = store.startActionEventStore()
    const listener = vi.fn()
    store.subscribeToActionEvents(listener)

    emit(started())
    emit(delta(2, 'A'))
    emit(delta(3, 'B'))
    expect(frameCallbacks.size).toBe(1)
    expect(vi.getTimerCount()).toBe(1)

    await vi.advanceTimersByTimeAsync(32)
    expect(store.getActionEventSnapshot().content).toBe('AB')
    expect(listener).toHaveBeenCalledTimes(3)
    expect(frameCallbacks.size).toBe(0)
    expect(vi.getTimerCount()).toBe(0)
    await vi.runAllTimersAsync()
    expect(listener).toHaveBeenCalledTimes(3)
    stop()
  })

  it('cancels the timeout when a frame wins and reuses one latch for held carry', async () => {
    const store = await import('./actionEventStore')
    const stop = store.startActionEventStore()
    const listener = vi.fn()
    store.subscribeToActionEvents(listener)

    emit(started())
    emit(delta(2, 'A'))
    emit(delta(3, 'BC'))
    runNextFrame()

    expect(store.getActionEventSnapshot().content).toBe('AB')
    expect(listener).toHaveBeenCalledTimes(3)
    expect(frameCallbacks.size).toBe(1)
    expect(vi.getTimerCount()).toBe(1)

    runNextFrame(32)
    expect(store.getActionEventSnapshot().content).toBe('ABC')
    expect(listener).toHaveBeenCalledTimes(4)
    expect(frameCallbacks.size).toBe(0)
    expect(vi.getTimerCount()).toBe(0)
    stop()
  })

  it('flushes synchronously on pageshow, visible resume, and native reveal', async () => {
    const store = await import('./actionEventStore')
    const stop = store.startActionEventStore()

    emit(started())
    emit(delta(2, 'A'))
    emit(delta(3, 'B'))
    window.dispatchEvent(new PageTransitionEvent('pageshow'))
    expect(store.getActionEventSnapshot().content).toBe('AB')
    expect(frameCallbacks.size).toBe(0)
    expect(vi.getTimerCount()).toBe(0)

    emit(delta(4, 'C'))
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      value: 'visible'
    })
    document.dispatchEvent(new Event('visibilitychange'))
    expect(store.getActionEventSnapshot().content).toBe('ABC')
    expect(frameCallbacks.size).toBe(0)
    expect(vi.getTimerCount()).toBe(0)

    emit(delta(5, 'D'))
    store.flushPendingActionEvents('native-reveal')
    expect(store.getActionEventSnapshot().content).toBe('ABCD')
    expect(frameCallbacks.size).toBe(0)
    expect(vi.getTimerCount()).toBe(0)
    stop()
  })
})

describe('action event store hydration and recovery', () => {
  it('atomically hydrates, replays a continuous buffered suffix, and ACKs the snapshot watermark', async () => {
    const store = await import('./actionEventStore')
    const recovery = vi.fn()
    const stop = store.startActionEventStore('session-1', recovery)
    const listener = vi.fn()
    store.subscribeToActionEvents(listener)

    emit(delta(12, 'A'))
    emit(delta(13, 'B'))
    const outcome = store.hydrateActionEventStore(snapshot({
      content: 'base',
      contentScalarCount: 4,
      lastContentSequence: 11,
      lastSequence: 11
    }))

    expect(outcome).toEqual({
      ack: {
        sessionId: 'session-1',
        sessionGeneration: 9,
        requestGeneration: 1,
        lastSequence: 11,
        handshakeGeneration: 4
      },
      recoveryNeeded: false
    })
    expect(store.getActionEventSnapshot().content).toBe('baseAB')
    expect(listener).toHaveBeenCalledTimes(2)
    expect(recovery).not.toHaveBeenCalled()
    stop()
  })

  it('discards buffered duplicates at or below the snapshot watermark', async () => {
    const store = await import('./actionEventStore')
    const stop = store.startActionEventStore('session-1')

    emit(delta(10, 'old-10'))
    emit(delta(11, 'old-11'))
    emit(delta(12, 'new'))
    const outcome = store.hydrateActionEventStore(snapshot({
      content: 'base',
      contentScalarCount: 4,
      lastContentSequence: 11,
      lastSequence: 11
    }))

    expect(outcome.recoveryNeeded).toBe(false)
    expect(store.getActionEventSnapshot().content).toBe('basenew')
    stop()
  })

  it('starts one recovery for a live sequence gap and buffers later events', async () => {
    const store = await import('./actionEventStore')
    const recovery = vi.fn()
    const stop = store.startActionEventStore('session-1', recovery)
    store.hydrateActionEventStore(snapshot({ lastSequence: 13 }))

    emit(delta(15, 'gap'))
    emit(delta(14, 'later-arrival'))

    expect(store.getActionEventSnapshot().content).toBe('')
    expect(recovery).toHaveBeenCalledTimes(1)
    expect(recovery).toHaveBeenCalledWith('sequence-gap')
    stop()
  })

  it('starts resync recovery and buffers subsequent live events', async () => {
    const store = await import('./actionEventStore')
    const recovery = vi.fn()
    const stop = store.startActionEventStore('session-1', recovery)
    store.hydrateActionEventStore(snapshot({ lastSequence: 11 }))

    emit({
      ...eventBase,
      type: 'resyncRequired',
      sequence: 12,
      snapshotLastSequence: 11
    })
    emit(delta(13, 'buffered'))

    expect(recovery).toHaveBeenCalledTimes(1)
    expect(recovery).toHaveBeenCalledWith('resync-required')
    expect(store.getActionEventSnapshot().content).toBe('')
    stop()
  })

  it('retains a higher session generation for recovery even when its sequence is lower', async () => {
    const store = await import('./actionEventStore')
    const recovery = vi.fn()
    const stop = store.startActionEventStore('session-1', recovery)

    emit(started(1, { sessionGeneration: 10 }))
    const outcome = store.hydrateActionEventStore(snapshot({
      sessionGeneration: 9,
      lastSequence: 11
    }))

    expect(outcome.recoveryNeeded).toBe(true)
    expect(recovery).not.toHaveBeenCalled()
    stop()
  })

  it('ignores an older request generation even when its sequence is newer', async () => {
    const store = await import('./actionEventStore')
    const stop = store.startActionEventStore('session-1')
    store.hydrateActionEventStore(snapshot({ lastSequence: 10 }))

    emit(started(11, { requestId: 'request-2', requestGeneration: 2 }))
    const current = store.getActionEventSnapshot()
    emit({
      ...eventBase,
      requestId: 'request-1',
      requestGeneration: 1,
      type: 'notice',
      sequence: 12,
      code: 'old',
      message: 'old notice'
    })

    expect(store.getActionEventSnapshot()).toBe(current)
    stop()
  })

  it.each([
    { lastContentSequence: 1, contentScalarCount: 1 },
    { lastContentSequence: 2, contentScalarCount: 2 }
  ])('recovers instead of publishing a terminal with mismatched watermarks', async (watermark) => {
    const store = await import('./actionEventStore')
    const recovery = vi.fn()
    const stop = store.startActionEventStore(undefined, recovery)

    emit(started())
    emit(delta(2, 'A'))
    emit({
      ...eventBase,
      type: 'completed',
      sequence: 3,
      ...watermark
    })

    expect(store.getActionEventSnapshot().status).toBe('streaming')
    expect(recovery).toHaveBeenCalledTimes(1)
    expect(recovery).toHaveBeenCalledWith('sequence-gap')
    stop()
  })

  it('returns replay recovery intent only after installing a continuous prefix', async () => {
    const store = await import('./actionEventStore')
    const recovery = vi.fn()
    const stop = store.startActionEventStore('session-1', recovery)

    emit(delta(12, 'A'))
    emit(delta(14, 'C'))
    const outcome = store.hydrateActionEventStore(snapshot({ lastSequence: 11 }))

    expect(outcome.recoveryNeeded).toBe(true)
    expect(store.getActionEventSnapshot().content).toBe('A')
    expect(recovery).not.toHaveBeenCalled()

    const second = store.hydrateActionEventStore(snapshot({
      content: 'AB',
      contentScalarCount: 2,
      lastContentSequence: 13,
      lastSequence: 13,
      handshakeGeneration: 5
    }))
    expect(second.recoveryNeeded).toBe(false)
    expect(store.getActionEventSnapshot().content).toBe('ABC')
    stop()
  })

  it('validates snapshot scalar counts before publishing any hydration', async () => {
    const store = await import('./actionEventStore')
    const stop = store.startActionEventStore('session-1')
    const listener = vi.fn()
    store.subscribeToActionEvents(listener)

    expect(() => store.hydrateActionEventStore(snapshot({
      content: '😀𠮷',
      contentScalarCount: 2,
      lastContentSequence: 2,
      lastSequence: 2
    }))).not.toThrow()
    const valid = store.getActionEventSnapshot()

    expect(() => store.hydrateActionEventStore(snapshot({
      content: '😀𠮷',
      contentScalarCount: 3,
      lastContentSequence: 2,
      lastSequence: 2
    }))).toThrow()
    const understated = snapshot({
      content: `# heading\n${'😀'.repeat(16_384)}`,
      contentScalarCount: 16_384,
      lastContentSequence: 2,
      lastSequence: 2
    }) as ResultSessionSnapshot
    expect(() => store.hydrateActionEventStore(understated)).toThrow()
    expect(store.getActionEventSnapshot()).toBe(valid)
    expect(listener).toHaveBeenCalledTimes(1)
    stop()
  })

  it('rejects a snapshot for a different active session', async () => {
    const store = await import('./actionEventStore')
    const stop = store.startActionEventStore('session-1')
    expect(() => store.hydrateActionEventStore(snapshot({ sessionId: 'session-2' })))
      .toThrow(/session/i)
    stop()
  })
})

describe('action event store ordered notices', () => {
  it('advances only notice sequence state and recovers gaps through hydration', async () => {
    const store = await import('./actionEventStore')
    const recovery = vi.fn()
    const stop = store.startActionEventStore(undefined, recovery)
    const listener = vi.fn()
    store.subscribeToActionEvents(listener)

    emit(started(20))
    emit(delta(21, 'A'))
    emit(delta(22, 'B'))
    const pendingObject = store.getActionEventSnapshot()
    const logicalBeforeNotice = store.getActionEventLogicalRevision()
    expect(frameCallbacks.size).toBe(1)
    expect(vi.getTimerCount()).toBe(1)

    emit({
      ...eventBase,
      type: 'notice',
      sequence: 23,
      code: 'fallback',
      message: 'fallback applied'
    })
    const noticed = store.getActionEventSnapshot()
    expect(noticed).toMatchObject({
      content: 'A',
      contentScalarCount: 1,
      contentRevision: 1,
      status: 'streaming',
      generationNotice: 'fallback applied'
    })
    expect(noticed).not.toBe(pendingObject)
    expect(store.getActionEventLogicalRevision()).toBe(logicalBeforeNotice + 1)
    expect(frameCallbacks.size).toBe(1)
    expect(vi.getTimerCount()).toBe(1)

    const callsAfterNotice = listener.mock.calls.length
    emit({
      ...eventBase,
      type: 'notice',
      sequence: 23,
      code: 'fallback',
      message: 'fallback applied'
    })
    expect(store.getActionEventSnapshot()).toBe(noticed)
    expect(listener).toHaveBeenCalledTimes(callsAfterNotice)

    emit({
      ...eventBase,
      type: 'notice',
      sequence: 25,
      code: 'gap',
      message: 'gap notice'
    })
    expect(recovery).toHaveBeenCalledWith('sequence-gap')

    const outcome = store.hydrateActionEventStore(snapshot({
      content: 'AB',
      contentScalarCount: 2,
      lastContentSequence: 22,
      lastSequence: 24,
      generationNotice: { code: 'snapshot', message: 'snapshot notice' },
      handshakeGeneration: 5
    }))
    expect(outcome.recoveryNeeded).toBe(false)
    expect(store.getActionEventSnapshot()).toMatchObject({
      content: 'AB',
      contentScalarCount: 2,
      contentRevision: 1,
      status: 'streaming',
      generationNotice: 'gap notice'
    })

    const afterReplay = store.getActionEventSnapshot()
    emit({
      ...eventBase,
      requestId: 'old-request',
      requestGeneration: 0,
      type: 'notice',
      sequence: 26,
      code: 'old',
      message: 'old notice'
    })
    expect(store.getActionEventSnapshot()).toBe(afterReplay)

    emit(started(26, { requestId: 'request-2', requestGeneration: 2 }))
    expect(store.getActionEventSnapshot()).toMatchObject({
      requestId: 'request-2',
      generationNotice: '',
      content: ''
    })
    stop()
  })
})
