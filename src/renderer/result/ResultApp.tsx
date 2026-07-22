import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type JSX
} from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  ArrowRight,
  Check,
  ChevronDown,
  CircleAlert,
  Copy,
  LoaderCircle,
  Maximize2,
  Minimize2,
  Pin,
  RefreshCw,
  Square,
  X
} from 'lucide-react'
import {
  DEFAULT_PUBLIC_SETTINGS,
  DEFAULT_RESULT_FONT_SIZE,
  RESULT_FONT_SIZE_MAX,
  RESULT_FONT_SIZE_MIN,
  detectTranslationLanguage,
  type ActionRetryOptions,
  type PublicSettings,
  type ResultSessionSnapshot,
  type SupportedLocale
} from '../../shared'
import { ActionIcon } from '../components/ActionIcon'
import { runDetached } from '../lib/asyncEffects'
import { getErrorMessage } from '../lib/errors'
import {
  flushPendingActionEvents,
  getActionEventSnapshot,
  subscribeToActionEvents
} from './actionEventStore'
import { useAutoFollowOutput } from './autoFollowOutput'
import {
  appendUserTurn,
  beginAssistantTurn,
  isAskAction,
  patchStreamingAssistant,
  type TranscriptTurn
} from './conversationTranscript'
import { ResultOutput } from './ResultOutput'
import { useResultContentOverflow } from './resultContentOverflow'
import type { ResultSessionBootstrap } from './resultSessionBootstrap'

const LANGUAGE_CODES = { 'zh-CN': 'CN', 'en-US': 'EN' } as const
const RESULT_RESIZE_DIRECTIONS = [
  'NorthWest',
  'North',
  'NorthEast',
  'East',
  'SouthEast',
  'South',
  'SouthWest',
  'West'
] as const

type ResultResizeDirection = (typeof RESULT_RESIZE_DIRECTIONS)[number]

interface ModelRoute {
  providerId: string
  modelId: string
}

interface ModelChoice extends ModelRoute {
  value: string
  providerName: string
  modelName: string
  keyConfigured: boolean
}

function modelRouteValue(route: ModelRoute): string {
  return JSON.stringify([route.providerId, route.modelId])
}

function modelRouteFromSnapshot(snapshot: ResultSessionSnapshot): ModelRoute | null {
  const { providerId, modelId } = snapshot
  if (providerId === undefined && modelId === undefined) return null
  if (providerId === undefined || modelId === undefined) {
    throw new Error('validated result snapshot has an incomplete model route')
  }
  return { providerId, modelId }
}

function isEditableTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable)
}

function isInteractiveElement(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(
    'button, select, a, input, textarea, [contenteditable="true"], [data-no-drag]'
  ) !== null
}

function nodeIsWithin(container: HTMLElement, node: Node | null): boolean {
  if (!node) return false
  return container === node || container.contains(node.nodeType === Node.TEXT_NODE ? node.parentNode : node)
}

export function selectedTextWithin(
  container: HTMLElement | null,
  selection: Selection | null
): string | null {
  if (!container || !selection || selection.isCollapsed || selection.rangeCount === 0) return null
  const range = selection.getRangeAt(0)
  if (!nodeIsWithin(container, range.startContainer) || !nodeIsWithin(container, range.endContainer)) {
    return null
  }
  const text = selection.toString()
  return text.trim() ? text : null
}

export function resultFontSize(settings: PublicSettings | null): number {
  const configured = settings?.result.fontSize
  return typeof configured === 'number' && Number.isFinite(configured)
    ? Math.min(RESULT_FONT_SIZE_MAX, Math.max(RESULT_FONT_SIZE_MIN, configured))
    : DEFAULT_RESULT_FONT_SIZE
}

export function isWindowsResultRenderer(userAgent = window.navigator.userAgent): boolean {
  return /Windows/i.test(userAgent)
}

export async function waitForResultRevealFrames(
  scheduleFrame: (callback: FrameRequestCallback) => number = window.requestAnimationFrame
): Promise<void> {
  await new Promise<void>((resolve) => scheduleFrame(() => resolve()))
  await new Promise<void>((resolve) => scheduleFrame(() => resolve()))
}

export interface ResultAppProps {
  bootstrap?: ResultSessionBootstrap | null
}

export function ResultApp({ bootstrap = null }: ResultAppProps = {}): JSX.Element {
  const sessionId = bootstrap?.sessionId.trim() ?? ''
  if (!bootstrap || !sessionId) {
    return (
      <main className="result-window">
        <div className="result-error" role="alert">
          <CircleAlert size={20} />
          <div>
            <strong>结果会话无效</strong>
            <p>缺少有效的会话标识，无法显示结果窗口。</p>
          </div>
        </div>
      </main>
    )
  }
  return <ResultSessionApp sessionId={sessionId} bootstrap={bootstrap} />
}

function ResultSessionApp({
  sessionId,
  bootstrap
}: {
  sessionId: string
  bootstrap: ResultSessionBootstrap
}): JSX.Element {
  const windowsRenderer = useMemo(() => isWindowsResultRenderer(), [])
  const state = useSyncExternalStore(
    subscribeToActionEvents,
    getActionEventSnapshot,
    getActionEventSnapshot
  )
  const [settings, setSettings] = useState<PublicSettings>(DEFAULT_PUBLIC_SETTINGS)
  const settingsRevisionRef = useRef(0)
  const [session, setSession] = useState<ResultSessionSnapshot | null>(null)
  const [pinned, setPinned] = useState(false)
  const [showOriginal, setShowOriginal] = useState(false)
  const [turns, setTurns] = useState<TranscriptTurn[]>([])
  const [translationTarget, setTranslationTarget] = useState<SupportedLocale | null>(null)
  const [activeModelRoute, setActiveModelRoute] = useState<ModelRoute | null>(null)
  const [routeSwitching, setRouteSwitching] = useState(false)
  const [retrying, setRetrying] = useState(false)
  const [commandMessage, setCommandMessage] = useState('')
  const [copyComplete, setCopyComplete] = useState(false)
  const [followUpQuestion, setFollowUpQuestion] = useState('')
  const [followUpExpanded, setFollowUpExpanded] = useState(false)
  const [followUpSubmitting, setFollowUpSubmitting] = useState(false)
  const [revealCommitted, setRevealCommitted] = useState(false)
  const [bootstrapMessage, setBootstrapMessage] = useState('')
  const [bootstrapRetrying, setBootstrapRetrying] = useState(false)
  const contentRef = useRef<HTMLDivElement>(null)
  const contentInnerRef = useRef<HTMLDivElement>(null)
  const copyResetTimer = useRef<number | null>(null)
  const selectionFrame = useRef<number | null>(null)
  const retryInFlight = useRef(false)
  const bootstrapRetryInFlight = useRef(false)
  const disposedRef = useRef(false)
  const followUpComposing = useRef(false)
  const askOriginalDefaulted = useRef(false)
  const askSawStreaming = useRef(false)
  const pendingFollowUp = useRef<{ question: string; requestId: string | null } | null>(null)
  const latestResultState = useRef(state)
  latestResultState.current = state
  const handleScroll = useAutoFollowOutput(contentRef, contentInnerRef, state.requestId)
  const isWaitingForFirstContent = state.status === 'streaming' && !state.content
  const contentOverflow = useResultContentOverflow(
    contentRef,
    contentInnerRef,
    !isWaitingForFirstContent
  )

  const action = useMemo(
    () => settings?.actions.find((item) => item.id === (state.actionId ?? session?.actionId)),
    [session?.actionId, settings, state.actionId]
  )
  const isAsk = isAskAction(action?.kind)
  const followUpDisabled =
    followUpSubmitting ||
    state.status === 'streaming' ||
    (!isAsk && state.status !== 'completed') ||
    (isAsk && state.status !== 'completed' && state.status !== 'idle')

  const defaultModelRoute = useMemo<ModelRoute | null>(() => {
    if (!action || !('providerId' in action) || !action.providerId || !action.modelId) return null
    return { providerId: action.providerId, modelId: action.modelId }
  }, [action])

  const effectiveModelRoute = activeModelRoute ?? defaultModelRoute
  const modelGroups = useMemo(() => (
    settings?.providers
      .filter((provider) => provider.models.length > 0)
      .map((provider) => ({
        id: provider.id,
        name: provider.name,
        keyConfigured: provider.keyConfigured,
        choices: provider.models.map<ModelChoice>((model) => ({
          providerId: provider.id,
          modelId: model.id,
          providerName: provider.name,
          modelName: model.name,
          keyConfigured: provider.keyConfigured,
          value: modelRouteValue({ providerId: provider.id, modelId: model.id })
        }))
      })) ?? []
  ), [settings?.providers])
  const modelChoices = useMemo(
    () => modelGroups.flatMap((provider) => provider.choices),
    [modelGroups]
  )
  const selectedModel = useMemo(() => {
    if (!effectiveModelRoute) return null
    return modelChoices.find((choice) =>
      choice.providerId === effectiveModelRoute.providerId &&
      choice.modelId === effectiveModelRoute.modelId
    ) ?? null
  }, [effectiveModelRoute, modelChoices])
  const modelSummary = selectedModel
    ? `${selectedModel.providerName} · ${selectedModel.modelName}`
    : '未选择模型'

  const reconcilePendingFollowUp = useCallback(
    (pending: { question: string; requestId: string | null }): void => {
      const latest = latestResultState.current
      if (pending.requestId && latest.requestId !== pending.requestId) return
      if (latest.status === 'completed') {
        pendingFollowUp.current = null
        setFollowUpQuestion((current) => current === pending.question ? '' : current)
      } else if (latest.status === 'error' || latest.status === 'cancelled') {
        setFollowUpQuestion((current) => current || pending.question)
      }
    },
    []
  )

  const closeWindow = useCallback(async (): Promise<void> => {
    await window.textLens.closeResult(sessionId)
  }, [sessionId])

  const cancel = useCallback(async (): Promise<void> => {
    setCommandMessage('')
    try {
      await window.textLens.cancelAction(sessionId)
    } catch (error) {
      setCommandMessage(getErrorMessage(error, '无法取消当前操作'))
    }
  }, [sessionId])

  const retry = useCallback(async (options?: ActionRetryOptions): Promise<boolean> => {
    if (retryInFlight.current) return false
    retryInFlight.current = true
    setRetrying(true)
    setCommandMessage('')
    try {
      const result = await window.textLens.retryAction(sessionId, options)
      if (!result.accepted) {
        setCommandMessage(result.message)
        return false
      }
      if (
        options?.targetLanguage !== undefined ||
        options?.providerId !== undefined ||
        options?.modelId !== undefined
      ) {
        pendingFollowUp.current = null
        setFollowUpQuestion('')
      } else if (pendingFollowUp.current) {
        pendingFollowUp.current.requestId = result.requestId ?? null
        reconcilePendingFollowUp(pendingFollowUp.current)
      }
      return true
    } catch (error) {
      setCommandMessage(getErrorMessage(error, '无法重试当前操作'))
      return false
    } finally {
      retryInFlight.current = false
      setRetrying(false)
    }
  }, [reconcilePendingFollowUp, sessionId])

  const switchModel = useCallback(async (value: string): Promise<void> => {
    const next = modelChoices.find((choice) => choice.value === value)
    if (
      !next || routeSwitching || state.status === 'streaming' ||
      (effectiveModelRoute?.providerId === next.providerId &&
        effectiveModelRoute.modelId === next.modelId)
    ) return

    const previous = activeModelRoute
    setRouteSwitching(true)
    setActiveModelRoute({ providerId: next.providerId, modelId: next.modelId })
    const accepted = await retry({ providerId: next.providerId, modelId: next.modelId })
    if (!accepted) setActiveModelRoute(previous)
    setRouteSwitching(false)
  }, [activeModelRoute, effectiveModelRoute, modelChoices, retry, routeSwitching, state.status])

  const copy = useCallback(async (): Promise<void> => {
    if (!state.content) return
    setCommandMessage('')
    try {
      await window.textLens.copyText(state.content)
      setCopyComplete(true)
      if (copyResetTimer.current !== null) window.clearTimeout(copyResetTimer.current)
      copyResetTimer.current = window.setTimeout(() => {
        copyResetTimer.current = null
        setCopyComplete(false)
      }, 1_500)
    } catch (error) {
      setCommandMessage(getErrorMessage(error, '复制失败'))
    }
  }, [state.content])

  const submitFollowUp = useCallback(async (): Promise<void> => {
    const question = followUpQuestion.trim()
    const canSubmit =
      isAsk
        ? state.status === 'completed' || state.status === 'idle'
        : state.status === 'completed'
    if (!question || followUpSubmitting || !canSubmit) return
    setCommandMessage('')
    setFollowUpSubmitting(true)
    try {
      if (!window.textLens.continueAction) {
        throw new Error('当前版本不支持继续提问')
      }
      if (isAsk) {
        const turnId =
          typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
            ? crypto.randomUUID()
            : `assistant-${Date.now()}`
        askSawStreaming.current = false
        setTurns((current) => beginAssistantTurn(appendUserTurn(current, question), turnId))
      }
      const result = await window.textLens.continueAction(sessionId, question)
      if (!result.accepted) {
        setCommandMessage(result.message)
        if (isAsk) {
          askSawStreaming.current = false
          setTurns((current) => (current.length < 2 ? current : current.slice(0, -2)))
        }
        return
      }
      const pending = { question, requestId: result.requestId ?? null }
      pendingFollowUp.current = pending
      setFollowUpQuestion('')
      reconcilePendingFollowUp(pending)
    } catch (error) {
      setCommandMessage(getErrorMessage(error, '无法继续提问'))
      if (isAsk) {
        askSawStreaming.current = false
        setTurns((current) => (current.length < 2 ? current : current.slice(0, -2)))
      }
    } finally {
      setFollowUpSubmitting(false)
    }
  }, [followUpQuestion, followUpSubmitting, isAsk, reconcilePendingFollowUp, sessionId, state.status])

  useEffect(() => {
    let disposed = false
    const startedAtRevision = settingsRevisionRef.current
    const unsubscribe = window.textLens.onSettingsChanged((next) => {
      settingsRevisionRef.current += 1
      setSettings(next)
    })
    void window.textLens.getSettings().then((initial) => {
      if (!disposed && settingsRevisionRef.current === startedAtRevision) {
        setSettings(initial)
      }
    }).catch((error: unknown) => {
      if (!disposed) {
        setCommandMessage(getErrorMessage(error, '无法读取结果窗口设置'))
      }
    })
    return () => {
      disposed = true
      unsubscribe()
    }
  }, [sessionId])

  const consumeBootstrap = useCallback(async (
    start: () => Promise<ResultSessionSnapshot | null>,
    isDisposed: () => boolean
  ): Promise<void> => {
    let snapshot: ResultSessionSnapshot | null
    try {
      snapshot = await start()
      if (isDisposed()) return
      if (!snapshot) throw new Error('结果会话不可用')
      setSession(snapshot)
      setPinned(snapshot.pinned)
      setActiveModelRoute(modelRouteFromSnapshot(snapshot))
      setBootstrapMessage('')
    } catch {
      if (!isDisposed()) {
        setBootstrapMessage('无法连接结果会话，请重试。')
      }
      return
    }

    try {
      await bootstrap.reveal(async () => {
        if (window.textLens.prepareResultReveal) {
          await window.textLens.prepareResultReveal(sessionId)
        }
        await waitForResultRevealFrames()
        if (window.textLens.commitResultReveal) {
          await window.textLens.commitResultReveal(sessionId)
        }
      })
      if (isDisposed()) return
      flushPendingActionEvents('native-reveal')
      setRevealCommitted(true)
    } catch {
      if (isDisposed()) return
      const revealMessage = '结果窗口显示失败，请重试。'
      setCommandMessage(revealMessage)
      runDetached(window.textLens.failResultReveal?.(sessionId, revealMessage), {
        scope: 'result',
        operation: 'fail-result-reveal',
        onError: (error) => setCommandMessage(getErrorMessage(error, revealMessage))
      })
    }
  }, [bootstrap, sessionId])

  useEffect(() => {
    let disposed = false
    disposedRef.current = false
    runDetached(consumeBootstrap(bootstrap.start, () => disposed), {
      scope: 'result',
      operation: 'result-ready-consume'
    })
    return () => {
      disposed = true
      disposedRef.current = true
    }
  }, [bootstrap, consumeBootstrap])

  const retryBootstrap = useCallback((): void => {
    if (bootstrapRetryInFlight.current) return
    bootstrapRetryInFlight.current = true
    setBootstrapRetrying(true)
    runDetached(consumeBootstrap(bootstrap.retryStart, () => disposedRef.current).finally(() => {
      bootstrapRetryInFlight.current = false
      if (!disposedRef.current) setBootstrapRetrying(false)
    }), {
      scope: 'result',
      operation: 'result-ready-retry',
      onError: (error) => setCommandMessage(getErrorMessage(error, '无法连接结果会话，请重试。'))
    })
  }, [bootstrap, consumeBootstrap])

  useEffect(() => {
    setTurns([])
    askOriginalDefaulted.current = false
    askSawStreaming.current = false
  }, [sessionId])

  useEffect(() => {
    if (!isAsk || askOriginalDefaulted.current) return
    askOriginalDefaulted.current = true
    setShowOriginal(true)
  }, [isAsk, sessionId])

  useEffect(() => {
    if (!state.requestId) return
    setCommandMessage('')
    setCopyComplete(false)
    if (!isAsk) setShowOriginal(false)
    if (copyResetTimer.current !== null) {
      window.clearTimeout(copyResetTimer.current)
      copyResetTimer.current = null
    }
  }, [isAsk, state.requestId])

  useEffect(() => {
    if (!isAsk) return
    if (state.status === 'streaming') {
      askSawStreaming.current = true
      setTurns((current) => patchStreamingAssistant(current, state.content, true))
      return
    }
    if (
      askSawStreaming.current &&
      (state.status === 'completed' ||
        state.status === 'cancelled' ||
        state.status === 'error')
    ) {
      setTurns((current) => patchStreamingAssistant(current, state.content, false))
      askSawStreaming.current = false
    }
  }, [isAsk, state.content, state.requestId, state.status])

  useEffect(() => {
    const pending = pendingFollowUp.current
    if (pending) reconcilePendingFollowUp(pending)
  }, [reconcilePendingFollowUp, state.requestId, state.status])

  useEffect(() => {
    return () => {
      if (copyResetTimer.current !== null) window.clearTimeout(copyResetTimer.current)
      if (selectionFrame.current !== null) window.cancelAnimationFrame(selectionFrame.current)
    }
  }, [])

  const keyboardState = useRef({ state, cancel, closeWindow, retry, copy })
  keyboardState.current = { state, cancel, closeWindow, retry, copy }
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (isEditableTarget(event.target) || event.metaKey || event.ctrlKey || event.altKey) return
      const current = keyboardState.current
      const key = event.key.toLowerCase()
      if (key === 'escape') {
        event.preventDefault()
        if (current.state.status === 'streaming') void current.cancel()
        else runDetached(current.closeWindow(), {
          scope: 'result',
          operation: 'close',
          onError: (error) => setCommandMessage(getErrorMessage(error, '无法关闭结果窗口'))
        })
        return
      }
      if (key === 'r' && current.state.status !== 'streaming') {
        event.preventDefault()
        void current.retry()
        return
      }
      if (key === 'c' && current.state.content && !window.getSelection()?.toString()) {
        event.preventDefault()
        void current.copy()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  useEffect(() => {
    if (action?.kind !== 'translate' || !session?.selection || !settings) {
      setTranslationTarget(null)
      return
    }
    const detected = detectTranslationLanguage(session.selection.text)
    setTranslationTarget(
      detected === settings.translate.primaryLanguage
        ? settings.translate.alternateLanguage
        : settings.translate.primaryLanguage
    )
  }, [action?.kind, session?.sessionId, session?.selection, settings?.translate])

  const openExternal = useCallback(async (url: string): Promise<void> => {
    setCommandMessage('')
    try {
      await window.textLens.openExternal(url)
    } catch (error) {
      setCommandMessage(getErrorMessage(error, '无法打开外部链接'))
    }
  }, [])

  const togglePin = async (): Promise<void> => {
    const next = !pinned
    setCommandMessage('')
    setPinned(next)
    try {
      if (window.textLens.setResultPinned) {
        await window.textLens.setResultPinned(sessionId, next)
      }
    } catch (error) {
      setPinned(!next)
      setCommandMessage(getErrorMessage(error, '无法更改置顶状态'))
    }
  }

  const pointerEnter = (): void => {
    runDetached(window.textLens.setResultPointerInside?.(sessionId, true), {
      scope: 'result',
      operation: 'pointer-inside'
    })
  }

  const pointerLeave = (): void => {
    runDetached(window.textLens.setResultPointerInside?.(sessionId, false), {
      scope: 'result',
      operation: 'pointer-outside'
    })
  }

  const startWindowDrag = (event: ReactPointerEvent<HTMLElement>): void => {
    if (event.button !== 0 || !event.isPrimary || isInteractiveElement(event.target)) return
    event.preventDefault()
    runDetached(getCurrentWindow().startDragging(), {
      scope: 'result',
      operation: 'start-dragging'
    })
  }

  const showSelectionToolbar = (event: ReactPointerEvent<HTMLDivElement>): void => {
    if (event.button !== 0 || !event.isPrimary || isInteractiveElement(event.target)) return
    const cursor = { x: event.screenX, y: event.screenY }
    if (selectionFrame.current !== null) window.cancelAnimationFrame(selectionFrame.current)
    selectionFrame.current = window.requestAnimationFrame(() => {
      selectionFrame.current = null
      const text = selectedTextWithin(contentRef.current, window.getSelection())
      if (!text || !window.textLens.showResultSelection) return
      void window.textLens
        .showResultSelection(sessionId, text, cursor)
        .catch((error: unknown) => {
          setCommandMessage(getErrorMessage(error, '无法处理所选文字'))
        })
    })
  }

  const selection = session?.selection
  const translationRoute = useMemo(() => {
    if (action?.kind !== 'translate' || !selection || !settings) return null
    const detected = detectTranslationLanguage(selection.text)
    const defaultTarget = detected === settings.translate.primaryLanguage
      ? settings.translate.alternateLanguage
      : settings.translate.primaryLanguage
    return { detected, target: translationTarget ?? defaultTarget }
  }, [action?.kind, selection, settings, translationTarget])
  const translationSwitching = state.status === 'streaming' || routeSwitching || retrying
  const changeTranslationTarget = (target: SupportedLocale): void => {
    const previous = translationTarget
    setRouteSwitching(true)
    setTranslationTarget(target)
    void retry({ targetLanguage: target }).then((accepted) => {
      if (!accepted) setTranslationTarget(previous)
    }).finally(() => setRouteSwitching(false))
  }

  const style = {
    '--result-font-size': `${resultFontSize(settings)}px`,
    '--result-scrollbar-track-margin': `${contentOverflow.trackMarginPx}px`
  } as CSSProperties

  const startWindowResize = (
    event: ReactPointerEvent<HTMLDivElement>,
    direction: ResultResizeDirection
  ): void => {
    if (event.button !== 0 || !event.isPrimary) return
    event.preventDefault()
    event.stopPropagation()
    runDetached(getCurrentWindow().startResizeDragging(direction), {
      scope: 'result',
      operation: 'start-resize-dragging'
    })
  }

  return (
    <main
      className={`result-window ${windowsRenderer ? 'result-window--windows' : ''} ${
        followUpExpanded ? 'result-window--followup-expanded' : ''
      }`}
      style={style}
      onPointerEnter={pointerEnter}
      onPointerLeave={pointerLeave}
    >
      {windowsRenderer && RESULT_RESIZE_DIRECTIONS.map((direction) => (
        <div
          key={direction}
          className={`result-resize-handle result-resize-handle--${direction.toLowerCase()}`}
          data-no-drag
          data-resize-direction={direction}
          aria-hidden="true"
          onPointerDown={(event) => startWindowResize(event, direction)}
        />
      ))}
      <header className="result-header" onPointerDown={startWindowDrag}>
        <div className="result-heading">
          <span className="result-heading__icon">
            <ActionIcon name={action?.icon ?? 'sparkles'} size={16} />
          </span>
          <div className="result-heading__line">
            <h1>{action?.name ?? 'AI 结果'}</h1>
            {translationRoute ? (
              <div className="translation-route" aria-label="翻译方向" data-no-drag>
                {windowsRenderer ? (
                  <>
                    <span className="translation-route__code">{LANGUAGE_CODES[translationRoute.detected]}</span>
                    <ArrowRight className="translation-route__arrow" size={10} />
                    <span
                      className="translation-route__target"
                      data-disabled={translationSwitching ? 'true' : undefined}
                    >
                      <span aria-hidden="true">{LANGUAGE_CODES[translationRoute.target]}</span>
                      <ChevronDown size={9} aria-hidden="true" />
                      <select
                        className="translation-route__native-select"
                        aria-label="翻译目标语言"
                        value={translationRoute.target}
                        disabled={translationSwitching}
                        onChange={(event) => changeTranslationTarget(
                          event.target.value as SupportedLocale
                        )}
                      >
                        <option value="zh-CN">CN</option>
                        <option value="en-US">EN</option>
                      </select>
                    </span>
                  </>
                ) : (
                  <>
                    <span>{LANGUAGE_CODES[translationRoute.detected]}</span><ArrowRight size={11} />
                    <select
                      aria-label="翻译目标语言"
                      value={translationRoute.target}
                      disabled={translationSwitching}
                      onChange={(event) => changeTranslationTarget(
                        event.target.value as SupportedLocale
                      )}
                    >
                      <option value="zh-CN">CN</option>
                      <option value="en-US">EN</option>
                    </select>
                  </>
                )}
              </div>
            ) : null}
            {effectiveModelRoute && modelChoices.length > 0 ? (
              <label className="result-model-switch" title={modelSummary} data-no-drag>
                <select
                  aria-label="切换模型"
                  value={modelRouteValue(effectiveModelRoute)}
                  disabled={state.status === 'streaming' || routeSwitching || retrying}
                  onChange={(event) => void switchModel(event.target.value)}
                >
                  {!selectedModel && <option value={modelRouteValue(effectiveModelRoute)}>未选择模型</option>}
                  {modelGroups.map((provider) => (
                    <optgroup
                      key={provider.id}
                      label={provider.keyConfigured ? provider.name : `${provider.name}（未配置密钥）`}
                      disabled={!provider.keyConfigured}
                    >
                      {provider.choices.map((choice) => (
                        <option key={choice.value} value={choice.value}>{choice.modelName}</option>
                      ))}
                    </optgroup>
                  ))}
                </select>
              </label>
            ) : (
              <span className="result-model-summary" title={modelSummary}>{modelSummary}</span>
            )}
          </div>
        </div>
        <div className="result-window-actions" data-no-drag>
          <button className={`icon-button result-pin ${pinned ? 'result-pin--active' : ''}`} type="button"
            aria-label={pinned ? '取消置顶' : '置顶结果窗口'} aria-pressed={pinned}
            title={pinned ? '取消置顶' : '置顶'} onClick={() => void togglePin()}>
            <Pin size={16} className={pinned ? 'result-pin__icon--active' : ''} />
          </button>
          <button className="icon-button result-close" type="button" aria-label="关闭结果窗口"
            title="关闭" onClick={() => runDetached(closeWindow(), {
              scope: 'result',
              operation: 'close',
              onError: (error) => setCommandMessage(getErrorMessage(error, '无法关闭结果窗口'))
            })}>
            <X size={15} aria-hidden="true" />
          </button>
        </div>
      </header>

      <div
        ref={contentRef}
        className={`result-content ${contentOverflow.isOverflowing ? 'result-content--scrollable' : ''}`}
        onScroll={handleScroll}
        onPointerUp={showSelectionToolbar}
      >
        <div ref={contentInnerRef} className="result-content__inner">
          {selection && (
            <section className="result-original">
              <button type="button" onClick={() => setShowOriginal((current) => !current)} aria-expanded={showOriginal}>
                <span>{showOriginal ? '隐藏原文' : '显示原文'}</span>
                <ChevronDown size={14} className={showOriginal ? 'is-expanded' : ''} />
              </button>
              {showOriginal && <div className="result-original__content">{selection.text}</div>}
            </section>
          )}

          {isAsk ? (
            <>
              {turns.length === 0 && state.status !== 'streaming' && state.status !== 'error' && (
                <div className="result-placeholder">
                  <span>已载入选中文本。请在下方输入问题。</span>
                </div>
              )}
              {turns.map((turn) => (
                <div
                  key={turn.id}
                  className={`result-turn result-turn--${turn.role}`}
                  data-role={turn.role}
                >
                  <div className="result-turn__label">{turn.role === 'user' ? '你' : 'AI'}</div>
                  {turn.role === 'user' ? (
                    <div className="result-turn__content">{turn.content}</div>
                  ) : turn.streaming || !turn.content ? (
                    turn.content ? (
                      <div className="result-turn__content stream-plain-text">{turn.content}</div>
                    ) : (
                      <div className="result-turn__waiting">
                        <LoaderCircle className="result-spin" size={16} />
                        <span>正在等待模型响应…</span>
                      </div>
                    )
                  ) : (
                    <article className="markdown-body result-turn__content">
                      <ResultOutput
                        requestKey={`${state.sessionGeneration ?? 'none'}:${state.requestGeneration ?? 'none'}:${turn.id}`}
                        status="completed"
                        content={turn.content}
                        contentScalarCount={turn.content.length}
                        contentRevision={1}
                        revealCommitted={revealCommitted}
                        onOpenExternal={openExternal}
                      />
                    </article>
                  )}
                </div>
              ))}
            </>
          ) : state.content ? (
            <article className="markdown-body">
              <ResultOutput
                requestKey={`${state.sessionGeneration ?? 'none'}:${state.requestGeneration ?? 'none'}:${state.requestId ?? 'none'}`}
                status={state.status}
                content={state.content}
                contentScalarCount={state.contentScalarCount}
                contentRevision={state.contentRevision}
                revealCommitted={revealCommitted}
                onOpenExternal={openExternal}
              />
            </article>
          ) : state.status === 'error' ? null : (
            <div className="result-placeholder">
              {state.status === 'streaming' ? <><LoaderCircle className="result-spin" size={24} /><span>正在等待模型响应…</span></>
                : <span>正在准备结果…</span>}
            </div>
          )}

          {state.status === 'error' && <div className="result-error" role="alert"><CircleAlert size={20} /><div>
            <strong>未能生成结果</strong><p>{state.errorMessage}</p></div></div>}
          {state.generationNotice && (
            <div className="result-inline-notice" role="status">{state.generationNotice}</div>
          )}
          {state.status === 'cancelled' && <div className="result-inline-notice" role="status">本次生成已取消。已生成的内容仍可复制。</div>}
        </div>
      </div>

      {bootstrapMessage && (
        <div className="result-command-error" role="alert">
          <span>{bootstrapMessage}</span>
          <button type="button" disabled={bootstrapRetrying} onClick={retryBootstrap}>
            重新连接结果会话
          </button>
        </div>
      )}
      {commandMessage && <div className="result-command-error" role="alert">{commandMessage}</div>}

      <footer className="result-footer">
        <div className={`result-followup ${followUpExpanded ? 'result-followup--expanded' : ''}`}>
          <textarea
            aria-label="继续提问"
            placeholder={isAsk ? '输入问题，基于选中文本提问' : '输入继续提问'}
            title="Enter 发送，Shift+Enter 换行"
            rows={followUpExpanded ? 4 : 1}
            maxLength={20_000}
            value={followUpQuestion}
            disabled={followUpDisabled}
            onChange={(event) => setFollowUpQuestion(event.target.value)}
            onCompositionStart={() => { followUpComposing.current = true }}
            onCompositionEnd={() => { followUpComposing.current = false }}
            onKeyDown={(event) => {
              if (
                event.key === 'Enter' &&
                !event.shiftKey &&
                !event.nativeEvent.isComposing &&
                !followUpComposing.current
              ) {
                event.preventDefault()
                void submitFollowUp()
              }
            }}
          />
          <button
            className="result-followup-expand"
            type="button"
            aria-label={followUpExpanded ? '收起继续提问输入框' : '放大继续提问输入框'}
            aria-expanded={followUpExpanded}
            title={followUpExpanded ? '收起输入框' : '放大输入框'}
            onClick={() => setFollowUpExpanded((current) => !current)}
          >
            {followUpExpanded ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
          </button>
        </div>
        <div className="result-actions">
          {!windowsRenderer && settings?.result.dismissMode === 'manual' && (
            <button className="result-footer-button" type="button" onClick={() => runDetached(closeWindow(), {
              scope: 'result',
              operation: 'close',
              onError: (error) => setCommandMessage(getErrorMessage(error, '无法关闭结果窗口'))
            })}>
              <X size={15} />关闭
            </button>
          )}
          {state.status === 'streaming' ? (
            <button className="result-footer-button" type="button" onClick={() => void cancel()}><Square size={13} fill="currentColor" />停止</button>
          ) : (
            <button className="result-footer-button" type="button" disabled={retrying || !state.requestId || (state.status === 'error' && !state.retryable)}
              onClick={() => void retry()}><RefreshCw size={14} />重试</button>
          )}
          <button className="result-footer-button" type="button" disabled={!state.content} onClick={() => void copy()}>
            {copyComplete ? <Check size={14} /> : <Copy size={14} />}{copyComplete ? '已复制' : '复制'}
          </button>
        </div>
      </footer>
    </main>
  )
}
