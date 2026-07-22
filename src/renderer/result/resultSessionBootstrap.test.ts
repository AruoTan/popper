import { describe, expect, it, vi } from 'vitest'

import type {
  ResultReadyAck,
  ResultSessionSnapshot
} from '../../shared'
import type { ResultHydrationOutcome } from './actionEventStore'
import {
  createResultSessionBootstrap,
  type ResultSessionBootstrapDependencies
} from './resultSessionBootstrap'

function snapshot(handshakeGeneration = 4): ResultSessionSnapshot {
  return {
    sessionId: 'session-1',
    sessionGeneration: 3,
    requestId: 'request-1',
    requestGeneration: 1,
    actionId: 'translate',
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
    lastSequence: 11,
    handshakeGeneration,
    errorMessage: '',
    retryable: false,
    pinned: false
  }
}

function ackFor(handshakeGeneration = 4): ResultReadyAck {
  return {
    sessionId: 'session-1',
    sessionGeneration: 3,
    requestGeneration: 1,
    lastSequence: 11,
    handshakeGeneration
  }
}

function deferred<T>(): {
  promise: Promise<T>
  resolve(value: T): void
  reject(error: unknown): void
} {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

function dependencies(overrides: Partial<ResultSessionBootstrapDependencies> = {}): {
  values: ResultSessionBootstrapDependencies
  deferredCallbacks: Array<() => void>
} {
  const deferredCallbacks: Array<() => void> = []
  const values: ResultSessionBootstrapDependencies = {
    beginResultReady: vi.fn().mockResolvedValue(snapshot()),
    hydrate: vi.fn().mockReturnValue({ ack: ackFor(), recoveryNeeded: false }),
    ackResultReady: vi.fn().mockResolvedValue(true),
    defer: (callback) => deferredCallbacks.push(callback),
    reportError: vi.fn(),
    ...overrides
  }
  return { values, deferredCallbacks }
}

describe('ResultSessionBootstrap', () => {
  it('shares a cached successful startup and acknowledges only after hydration', async () => {
    const { values } = dependencies()
    const bootstrap = createResultSessionBootstrap('session-1', values)

    const first = bootstrap.start()
    const second = bootstrap.start()

    expect(first).toBe(second)
    await expect(first).resolves.toEqual(snapshot())
    expect(values.beginResultReady).toHaveBeenCalledOnce()
    expect(values.hydrate).toHaveBeenCalledWith(snapshot())
    expect(values.ackResultReady).toHaveBeenCalledWith(ackFor())
    expect(
      (values.beginResultReady as ReturnType<typeof vi.fn>).mock.invocationCallOrder[0]
    ).toBeLessThan((values.hydrate as ReturnType<typeof vi.fn>).mock.invocationCallOrder[0]!)
    expect(
      (values.hydrate as ReturnType<typeof vi.fn>).mock.invocationCallOrder[0]
    ).toBeLessThan((values.ackResultReady as ReturnType<typeof vi.fn>).mock.invocationCallOrder[0]!)
  })

  it('waits for the acknowledgement before resolving startup', async () => {
    const acknowledged = deferred<boolean>()
    const { values } = dependencies({ ackResultReady: vi.fn(() => acknowledged.promise) })
    const bootstrap = createResultSessionBootstrap('session-1', values)
    let resolved = false
    const started = bootstrap.start().then(() => { resolved = true })

    await Promise.resolve()
    expect(resolved).toBe(false)
    acknowledged.resolve(true)
    await started
    expect(resolved).toBe(true)
  })

  it('accepts a null ready snapshot without hydrating or acknowledging', async () => {
    const { values } = dependencies({ beginResultReady: vi.fn().mockResolvedValue(null) })
    const bootstrap = createResultSessionBootstrap('session-1', values)

    await expect(bootstrap.start()).resolves.toBeNull()
    expect(values.hydrate).not.toHaveBeenCalled()
    expect(values.ackResultReady).not.toHaveBeenCalled()
  })

  it.each(['begin', 'ack'] as const)(
    'latches a rejected %s startup until retryStart explicitly retries it',
    async (stage) => {
      const beginResultReady = stage === 'begin'
        ? vi.fn().mockRejectedValueOnce(new Error('begin failed')).mockResolvedValue(snapshot(5))
        : vi.fn().mockResolvedValue(snapshot(5))
      const ackResultReady = stage === 'ack'
        ? vi.fn().mockRejectedValueOnce(new Error('ack failed')).mockResolvedValue(true)
        : vi.fn().mockResolvedValue(true)
      const { values } = dependencies({ beginResultReady, ackResultReady })
      const bootstrap = createResultSessionBootstrap('session-1', values)

      await expect(bootstrap.start()).rejects.toThrow(`${stage} failed`)
      await expect(bootstrap.start()).rejects.toThrow(`${stage} failed`)
      expect(beginResultReady).toHaveBeenCalledTimes(1)

      await expect(bootstrap.retryStart()).resolves.toEqual(snapshot(5))
      expect(beginResultReady).toHaveBeenCalledTimes(2)
    }
  )

  it('serializes a false ACK into one deferred fresh handshake shared by recovery callers', async () => {
    const firstAck = deferred<boolean>()
    const { values, deferredCallbacks } = dependencies({
      beginResultReady: vi.fn()
        .mockResolvedValueOnce(snapshot(4))
        .mockResolvedValueOnce(snapshot(5)),
      hydrate: vi.fn()
        .mockReturnValueOnce({ ack: ackFor(4), recoveryNeeded: false })
        .mockReturnValueOnce({ ack: ackFor(5), recoveryNeeded: false }),
      ackResultReady: vi.fn()
        .mockReturnValueOnce(firstAck.promise)
        .mockResolvedValueOnce(true)
    })
    const bootstrap = createResultSessionBootstrap('session-1', values)
    const started = bootstrap.start()
    const firstRecovery = bootstrap.recover()
    const secondRecovery = bootstrap.recover()

    expect(firstRecovery).toBe(secondRecovery)
    await Promise.resolve()
    expect(values.beginResultReady).toHaveBeenCalledTimes(1)

    firstAck.resolve(false)
    await Promise.resolve()
    await Promise.resolve()
    expect(deferredCallbacks).toHaveLength(1)
    expect(values.beginResultReady).toHaveBeenCalledTimes(1)

    deferredCallbacks.shift()!()
    await expect(started).resolves.toEqual(snapshot(5))
    await expect(firstRecovery).resolves.toEqual(snapshot(5))
    expect(values.beginResultReady).toHaveBeenCalledTimes(2)
  })

  it('coalesces concurrent fresh recovery handshakes and clears a rejected recovery', async () => {
    const recovery = deferred<ResultSessionSnapshot | null>()
    const { values } = dependencies({ beginResultReady: vi.fn(() => recovery.promise) })
    const bootstrap = createResultSessionBootstrap('session-1', values)
    const first = bootstrap.recover()
    const second = bootstrap.recover()

    expect(first).toBe(second)
    recovery.reject(new Error('recovery failed'))
    await expect(first).rejects.toThrow('recovery failed')

    ;(values.beginResultReady as ReturnType<typeof vi.fn>).mockResolvedValueOnce(snapshot(6))
    await expect(bootstrap.recover()).resolves.toEqual(snapshot(6))
    expect(values.beginResultReady).toHaveBeenCalledTimes(2)
  })

  it('defers hydration recovery until the acknowledged handshake has completed', async () => {
    const acknowledged = deferred<boolean>()
    const { values, deferredCallbacks } = dependencies({
      beginResultReady: vi.fn()
        .mockResolvedValueOnce(snapshot(4))
        .mockResolvedValueOnce(snapshot(5)),
      hydrate: vi.fn()
        .mockReturnValueOnce({ ack: ackFor(4), recoveryNeeded: true })
        .mockReturnValueOnce({ ack: ackFor(5), recoveryNeeded: false }),
      ackResultReady: vi.fn()
        .mockReturnValueOnce(acknowledged.promise)
        .mockResolvedValueOnce(true)
    })
    const bootstrap = createResultSessionBootstrap('session-1', values)
    const started = bootstrap.start()

    await Promise.resolve()
    expect(values.beginResultReady).toHaveBeenCalledOnce()
    acknowledged.resolve(true)
    await expect(started).resolves.toEqual(snapshot(4))
    expect(values.beginResultReady).toHaveBeenCalledOnce()
    expect(deferredCallbacks).toHaveLength(1)

    deferredCallbacks.shift()!()
    await Promise.resolve()
    expect(values.beginResultReady).toHaveBeenCalledTimes(2)
  })

  it('caches a native reveal Promise for all callers', async () => {
    const { values } = dependencies()
    const bootstrap = createResultSessionBootstrap('session-1', values)
    const run = vi.fn().mockResolvedValue(undefined)

    const first = bootstrap.reveal(run)
    const second = bootstrap.reveal(vi.fn())

    expect(first).toBe(second)
    await expect(first).resolves.toBeUndefined()
    expect(run).toHaveBeenCalledOnce()
  })
})
