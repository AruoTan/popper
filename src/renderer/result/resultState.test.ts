import type { ActionStreamEvent } from '../../shared'
import {
  appendResultDeltaBatch,
  INITIAL_RESULT_STATE,
  reduceActionEvent,
  resultStateFromSnapshot
} from './resultState'

describe('result action stream reducer', () => {
  const started = (requestGeneration: number, sequence: number): ActionStreamEvent => ({
    type: 'started',
    sessionId: 'session-1',
    sessionGeneration: 3,
    requestId: `request-${requestGeneration}`,
    requestGeneration,
    sequence,
    actionId: 'summary'
  })

  const terminalEvent = (
    type: 'completed' | 'cancelled' | 'error'
  ): ActionStreamEvent => {
    const base = {
      sessionId: 'session-1',
      sessionGeneration: 3,
      requestId: 'request-1',
      requestGeneration: 1,
      sequence: 3,
      actionId: 'summary'
    } as const
    if (type === 'completed') {
      return { ...base, type, lastContentSequence: 2, contentScalarCount: 7 }
    }
    if (type === 'cancelled') return { ...base, type }
    return { ...base, type, code: 'network', message: 'failed', retryable: true }
  }

  const lateDelta: ActionStreamEvent = {
    sessionId: 'session-1',
    sessionGeneration: 3,
    requestId: 'request-1',
    requestGeneration: 1,
    sequence: 4,
    actionId: 'summary',
    type: 'delta',
    delta: 'late'
  }

  const differentTerminal: ActionStreamEvent = {
    sessionId: 'session-1',
    sessionGeneration: 3,
    requestId: 'request-1',
    requestGeneration: 1,
    sequence: 5,
    actionId: 'summary',
    type: 'error',
    code: 'late-error',
    message: 'late error',
    retryable: false
  }

  it.each(['completed', 'cancelled', 'error'] as const)(
    'keeps %s terminal monotonic',
    (terminalType) => {
      let state = reduceActionEvent(INITIAL_RESULT_STATE, started(1, 1))
      state = appendResultDeltaBatch(state, {
        requestId: 'request-1',
        sessionGeneration: 3,
        requestGeneration: 1,
        delta: 'partial',
        contentScalarCount: 7
      })
      const terminal = reduceActionEvent(state, terminalEvent(terminalType))
      expect(reduceActionEvent(terminal, lateDelta)).toBe(terminal)
      expect(reduceActionEvent(terminal, differentTerminal)).toBe(terminal)
    }
  )

  it('ignores same and lower request generations then resets for a newer request', () => {
    let state = reduceActionEvent(INITIAL_RESULT_STATE, started(2, 1))
    state = appendResultDeltaBatch(state, {
      requestId: 'request-2',
      sessionGeneration: 3,
      requestGeneration: 2,
      delta: 'partial',
      contentScalarCount: 7
    })

    expect(reduceActionEvent(state, started(2, 2))).toBe(state)
    expect(reduceActionEvent(state, started(1, 3))).toBe(state)

    const next = reduceActionEvent(state, started(3, 4))
    expect(next).toMatchObject({
      requestId: 'request-3',
      requestGeneration: 3,
      status: 'streaming',
      content: '',
      contentScalarCount: 0,
      contentRevision: 0,
      generationNotice: ''
    })
  })

  it('only appends a matching streaming delta batch once', () => {
    const state = reduceActionEvent(INITIAL_RESULT_STATE, started(1, 1))
    const appended = appendResultDeltaBatch(state, {
      requestId: 'request-1',
      sessionGeneration: 3,
      requestGeneration: 1,
      delta: '😀',
      contentScalarCount: 1
    })

    expect(appended).toMatchObject({
      content: '😀',
      contentScalarCount: 1,
      contentRevision: 1
    })
    expect(appendResultDeltaBatch(appended, {
      requestId: 'other',
      sessionGeneration: 3,
      requestGeneration: 1,
      delta: 'ignored',
      contentScalarCount: 8
    })).toBe(appended)
  })

  it('preserves partial content for cancelled and error terminal states', () => {
    const streaming = appendResultDeltaBatch(
      reduceActionEvent(INITIAL_RESULT_STATE, started(1, 1)),
      {
        requestId: 'request-1',
        sessionGeneration: 3,
        requestGeneration: 1,
        delta: 'partial',
        contentScalarCount: 7
      }
    )

    expect(reduceActionEvent(streaming, terminalEvent('cancelled'))).toMatchObject({
      status: 'cancelled',
      content: 'partial',
      retryable: true
    })
    expect(reduceActionEvent(streaming, terminalEvent('error'))).toMatchObject({
      status: 'error',
      content: 'partial',
      errorMessage: 'failed',
      retryable: true
    })
  })

  it('stores a matching notice without changing body state', () => {
    const streaming = appendResultDeltaBatch(
      reduceActionEvent(INITIAL_RESULT_STATE, started(1, 1)),
      {
        requestId: 'request-1',
        sessionGeneration: 3,
        requestGeneration: 1,
        delta: 'partial',
        contentScalarCount: 7
      }
    )
    const notice: ActionStreamEvent = {
      sessionId: 'session-1',
      sessionGeneration: 3,
      requestId: 'request-1',
      requestGeneration: 1,
      sequence: 3,
      actionId: 'summary',
      type: 'notice',
      code: 'THINKING_CONTROL_FALLBACK',
      message: 'fallback applied'
    }

    const noticed = reduceActionEvent(streaming, notice)
    expect(noticed).toMatchObject({
      status: 'streaming',
      content: 'partial',
      contentScalarCount: 7,
      contentRevision: 1,
      generationNotice: 'fallback applied'
    })
    expect(reduceActionEvent(noticed, { ...notice, requestId: 'old', requestGeneration: 0 }))
      .toBe(noticed)
  })

  it('maps snapshot generations, scalar count, revision, and notice', () => {
    const state = resultStateFromSnapshot({
      sessionId: 'session-1',
      sessionGeneration: 3,
      requestId: 'request-1',
      requestGeneration: 1,
      actionId: 'summary',
      selection: {
        selectionId: 'selection-1',
        text: 'selected',
        sourceApp: { name: 'Test', bundleId: null },
        anchor: { kind: 'cursor', x: 1, y: 2 },
        direction: 'unknown',
        isFullscreen: false
      },
      status: 'streaming',
      content: '😀',
      contentScalarCount: 1,
      lastContentSequence: 2,
      lastSequence: 2,
      handshakeGeneration: 1,
      generationNotice: { code: 'fallback', message: 'fallback applied' },
      errorMessage: '',
      retryable: false,
      pinned: false
    })

    expect(state).toMatchObject({
      sessionGeneration: 3,
      requestGeneration: 1,
      contentScalarCount: 1,
      contentRevision: 1,
      generationNotice: 'fallback applied'
    })
  })
})
