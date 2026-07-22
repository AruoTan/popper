import {
  countUnicodeScalars,
  type ActionStreamEvent,
  type ResultReadyAck,
  type ResultSessionSnapshot,
  type Unsubscribe
} from '../../shared'
import {
  appendResultDeltaBatch,
  INITIAL_RESULT_STATE,
  reduceActionEvent,
  resultStateFromSnapshot,
  type ResultState
} from './resultState'
import { splitStableGraphemeTail } from './streamPlayback'

const NOTIFY_FALLBACK_MS = 32

type StorePhase = 'hydrating' | 'live' | 'recovering'
export type ActionEventRecoveryReason = 'sequence-gap' | 'resync-required'
export type ActionEventFlushReason =
  | 'frame'
  | 'timeout'
  | 'visibility'
  | 'pageshow'
  | 'native-reveal'

interface LogicalCursor {
  sessionGeneration: number
  requestGeneration: number
  lastSequence: number
  lastContentSequence: number
  contentScalarCount: number
}

interface PendingContent {
  chunks: string[]
  graphemeCarry: string
  carryInputRevision: number
  carryHeldAtRevision: number
}

export interface ResultHydrationOutcome {
  ack: ResultReadyAck
  recoveryNeeded: boolean
}

let publishedSnapshot = INITIAL_RESULT_STATE
let phase: StorePhase = 'hydrating'
let cursor: LogicalCursor | null = null
let pending = emptyPendingContent()
let bufferedEvents: ActionStreamEvent[] = []
let logicalRevision = 0
let hasPublishedNonEmptyContent = false

let stopBridgeListener: Unsubscribe | null = null
let activeSessionId: string | null = null
let onRecoveryRequired: ((reason: ActionEventRecoveryReason) => void) | null = null
let scheduledFrame: number | null = null
let scheduledTimer: number | null = null
const listeners = new Set<() => void>()

function emptyPendingContent(): PendingContent {
  return {
    chunks: [],
    graphemeCarry: '',
    carryInputRevision: 0,
    carryHeldAtRevision: -1
  }
}

function notifySubscribers(): void {
  for (const listener of listeners) listener()
}

function cancelNotificationLatch(): void {
  if (scheduledFrame !== null) window.cancelAnimationFrame(scheduledFrame)
  if (scheduledTimer !== null) window.clearTimeout(scheduledTimer)
  scheduledFrame = null
  scheduledTimer = null
}

function scheduleNotification(): void {
  if (scheduledFrame !== null || scheduledTimer !== null) return
  scheduledFrame = window.requestAnimationFrame(() => {
    flushPendingActionEvents('frame')
  })
  scheduledTimer = window.setTimeout(() => {
    flushPendingActionEvents('timeout')
  }, NOTIFY_FALLBACK_MS)
}

function materializePending(forceCarry: boolean): boolean {
  if (!cursor || !publishedSnapshot.requestId) return false

  const chunks = pending.chunks
  pending.chunks = []

  if (pending.graphemeCarry) {
    if (forceCarry || pending.carryHeldAtRevision === pending.carryInputRevision) {
      chunks.push(pending.graphemeCarry)
      pending.graphemeCarry = ''
      pending.carryHeldAtRevision = -1
    } else {
      pending.carryHeldAtRevision = pending.carryInputRevision
    }
  }

  const delta = chunks.join('')
  if (!delta) return false

  const next = appendResultDeltaBatch(publishedSnapshot, {
    requestId: publishedSnapshot.requestId,
    sessionGeneration: cursor.sessionGeneration,
    requestGeneration: cursor.requestGeneration,
    delta,
    contentScalarCount:
      publishedSnapshot.contentScalarCount + countUnicodeScalars(delta)
  })
  if (next === publishedSnapshot) return false
  publishedSnapshot = next
  return true
}

export function flushPendingActionEvents(reason: ActionEventFlushReason): void {
  cancelNotificationLatch()
  if (phase !== 'live') return

  const changed = materializePending(reason !== 'frame')
  if (changed) notifySubscribers()
  if (pending.graphemeCarry) scheduleNotification()
}

function publishReducedEvent(event: ActionStreamEvent, notify: boolean): boolean {
  const next = reduceActionEvent(publishedSnapshot, event)
  if (next === publishedSnapshot) return false
  publishedSnapshot = next
  if (notify) notifySubscribers()
  return true
}

function beginRecovery(
  reason: ActionEventRecoveryReason,
  event: ActionStreamEvent,
  reportRecovery: boolean
): void {
  if (phase !== 'live') {
    bufferedEvents.push(event)
    return
  }
  phase = 'recovering'
  cancelNotificationLatch()
  bufferedEvents.push(event)
  if (reportRecovery) onRecoveryRequired?.(reason)
}

function acceptInitialStarted(
  event: Extract<ActionStreamEvent, { type: 'started' }>,
  notify: boolean
): void {
  cancelNotificationLatch()
  pending = emptyPendingContent()
  cursor = {
    sessionGeneration: event.sessionGeneration,
    requestGeneration: event.requestGeneration,
    lastSequence: event.sequence,
    lastContentSequence: 0,
    contentScalarCount: 0
  }
  logicalRevision += 1
  hasPublishedNonEmptyContent = false
  publishReducedEvent(event, notify)
}

function acceptDelta(
  event: Extract<ActionStreamEvent, { type: 'delta' }>,
  notify: boolean,
  schedule: boolean
): void {
  if (!cursor) return
  cursor.lastSequence = event.sequence
  logicalRevision += 1
  if (!event.delta) return

  cursor.lastContentSequence = event.sequence
  cursor.contentScalarCount += countUnicodeScalars(event.delta)

  if (!hasPublishedNonEmptyContent) {
    const next = appendResultDeltaBatch(publishedSnapshot, {
      requestId: event.requestId,
      sessionGeneration: event.sessionGeneration,
      requestGeneration: event.requestGeneration,
      delta: event.delta,
      contentScalarCount: cursor.contentScalarCount
    })
    hasPublishedNonEmptyContent = true
    if (next !== publishedSnapshot) {
      publishedSnapshot = next
      if (notify) notifySubscribers()
    }
    return
  }

  const update = splitStableGraphemeTail(pending.graphemeCarry, event.delta)
  if (update.released) pending.chunks.push(update.released)
  pending.graphemeCarry = update.carry
  pending.carryInputRevision = logicalRevision
  if (schedule) scheduleNotification()
}

function acceptTerminal(
  event: Extract<ActionStreamEvent, { type: 'completed' | 'cancelled' | 'error' }>,
  notify: boolean,
  reportRecovery: boolean
): void {
  if (!cursor) return
  if (
    event.type === 'completed' &&
    (event.lastContentSequence !== cursor.lastContentSequence ||
      event.contentScalarCount !== cursor.contentScalarCount)
  ) {
    beginRecovery('sequence-gap', event, reportRecovery)
    return
  }

  cancelNotificationLatch()
  const contentChanged = materializePending(true)
  cursor.lastSequence = event.sequence
  logicalRevision += 1
  const stateBeforeTerminal = publishedSnapshot
  const terminalChanged = publishReducedEvent(event, false)
  if (notify && (contentChanged || terminalChanged || publishedSnapshot !== stateBeforeTerminal)) {
    notifySubscribers()
  }
}

function processLiveEvent(
  event: ActionStreamEvent,
  options: { notify: boolean; reportRecovery: boolean; schedule: boolean }
): void {
  if (!cursor) {
    if (event.type === 'started') {
      acceptInitialStarted(event, options.notify)
    } else {
      beginRecovery('sequence-gap', event, options.reportRecovery)
    }
    return
  }

  if (event.sessionGeneration < cursor.sessionGeneration) return
  if (event.sessionGeneration > cursor.sessionGeneration) {
    beginRecovery('sequence-gap', event, options.reportRecovery)
    return
  }
  if (event.requestGeneration < cursor.requestGeneration) return
  if (event.sequence <= cursor.lastSequence) return
  if (event.sequence !== cursor.lastSequence + 1) {
    beginRecovery('sequence-gap', event, options.reportRecovery)
    return
  }
  if (event.requestGeneration > cursor.requestGeneration && event.type !== 'started') {
    beginRecovery('sequence-gap', event, options.reportRecovery)
    return
  }
  if (
    event.requestGeneration === cursor.requestGeneration &&
    (event.requestId !== publishedSnapshot.requestId ||
      event.actionId !== publishedSnapshot.actionId)
  ) {
    beginRecovery('sequence-gap', event, options.reportRecovery)
    return
  }
  if (
    publishedSnapshot.status !== 'streaming' &&
    event.requestGeneration === cursor.requestGeneration
  ) return

  if (event.type === 'resyncRequired') {
    beginRecovery('resync-required', event, options.reportRecovery)
    return
  }
  if (event.type === 'started') {
    if (event.requestGeneration <= cursor.requestGeneration) {
      beginRecovery('sequence-gap', event, options.reportRecovery)
      return
    }
    acceptInitialStarted(event, options.notify)
    return
  }
  if (event.type === 'delta') {
    acceptDelta(event, options.notify, options.schedule)
    return
  }
  if (event.type === 'notice') {
    cursor.lastSequence = event.sequence
    logicalRevision += 1
    publishReducedEvent(event, options.notify)
    return
  }
  acceptTerminal(event, options.notify, options.reportRecovery)
}

function acceptActionEvent(event: ActionStreamEvent): void {
  if (activeSessionId && event.sessionId !== activeSessionId) return
  if (phase !== 'live') {
    bufferedEvents.push(event)
    return
  }
  processLiveEvent(event, { notify: true, reportRecovery: true, schedule: true })
}

function handleVisibilityChange(): void {
  if (document.visibilityState === 'visible') flushPendingActionEvents('visibility')
}

function handlePageShow(): void {
  flushPendingActionEvents('pageshow')
}

function isRecovering(): boolean {
  return phase === 'recovering'
}

/**
 * Installs the bridge listener before React mounts. A session-bound store starts
 * in hydration mode so native events remain ordered behind the ready snapshot.
 */
export function startActionEventStore(
  sessionId?: string | null,
  recoveryCallback?: (reason: ActionEventRecoveryReason) => void
): Unsubscribe {
  activeSessionId = sessionId || null
  onRecoveryRequired = recoveryCallback ?? null
  if (!sessionId && !cursor) phase = 'live'

  if (!stopBridgeListener) {
    const stopActionEvents = window.textLens.onActionEvent(acceptActionEvent)
    window.addEventListener('pageshow', handlePageShow)
    document.addEventListener('visibilitychange', handleVisibilityChange)
    stopBridgeListener = () => {
      stopActionEvents()
      window.removeEventListener('pageshow', handlePageShow)
      document.removeEventListener('visibilitychange', handleVisibilityChange)
    }
  }

  return () => {
    stopBridgeListener?.()
    stopBridgeListener = null
    cancelNotificationLatch()
    activeSessionId = null
    onRecoveryRequired = null
  }
}

export function getActionEventLogicalRevision(): number {
  return logicalRevision
}

/** Compatibility alias retained until Task 4 removes the old call site. */
export function getActionEventStoreRevision(): number {
  return getActionEventLogicalRevision()
}

export function hydrateActionEventStore(
  sessionSnapshot: ResultSessionSnapshot,
  _expectedEventRevision?: number
): ResultHydrationOutcome {
  if (activeSessionId && sessionSnapshot.sessionId !== activeSessionId) {
    throw new Error('Result session snapshot does not match the active session')
  }
  if (countUnicodeScalars(sessionSnapshot.content) !== sessionSnapshot.contentScalarCount) {
    throw new Error('Result session snapshot contentScalarCount is invalid')
  }

  const ack: ResultReadyAck = {
    sessionId: sessionSnapshot.sessionId,
    sessionGeneration: sessionSnapshot.sessionGeneration,
    requestGeneration: sessionSnapshot.requestGeneration,
    lastSequence: sessionSnapshot.lastSequence,
    handshakeGeneration: sessionSnapshot.handshakeGeneration
  }
  const eventsWaitingForSnapshot = bufferedEvents

  cancelNotificationLatch()
  publishedSnapshot = resultStateFromSnapshot(sessionSnapshot)
  cursor = {
    sessionGeneration: sessionSnapshot.sessionGeneration,
    requestGeneration: sessionSnapshot.requestGeneration,
    lastSequence: sessionSnapshot.lastSequence,
    lastContentSequence: sessionSnapshot.lastContentSequence,
    contentScalarCount: sessionSnapshot.contentScalarCount
  }
  pending = emptyPendingContent()
  bufferedEvents = []
  phase = 'live'
  hasPublishedNonEmptyContent = sessionSnapshot.contentScalarCount > 0
  logicalRevision += 1
  notifySubscribers()

  const replayCandidates = eventsWaitingForSnapshot.filter((event) => {
    if (event.sessionId !== sessionSnapshot.sessionId) return false
    if (event.sessionGeneration < sessionSnapshot.sessionGeneration) return false
    if (event.sessionGeneration > sessionSnapshot.sessionGeneration) return true
    if (
      event.sessionGeneration === sessionSnapshot.sessionGeneration &&
      event.requestGeneration < sessionSnapshot.requestGeneration
    ) return false
    return event.sequence > sessionSnapshot.lastSequence
  })
  const stateBeforeReplay = publishedSnapshot

  for (let index = 0; index < replayCandidates.length; index += 1) {
    const event = replayCandidates[index]!
    processLiveEvent(event, {
      notify: false,
      reportRecovery: false,
      schedule: false
    })
    if (isRecovering()) {
      bufferedEvents.push(...replayCandidates.slice(index + 1))
      break
    }
  }

  materializePending(true)
  if (publishedSnapshot !== stateBeforeReplay) notifySubscribers()

  return { ack, recoveryNeeded: isRecovering() }
}

export function subscribeToActionEvents(listener: () => void): Unsubscribe {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function getActionEventSnapshot(): ResultState {
  return publishedSnapshot
}
