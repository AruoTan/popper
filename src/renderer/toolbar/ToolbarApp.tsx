import {
  ChevronDown,
  CircleAlert,
  LoaderCircle,
  SendHorizontal,
  Settings2,
  Sparkles,
  X,
} from "lucide-react";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type JSX,
  type MouseEvent,
} from "react";

import { MAX_ENABLED_ACTIONS, type PublicSettings, type SelectionPayload } from "../../shared";
import { ActionIcon } from "../components/ActionIcon";
import { runDetached } from "../lib/asyncEffects";
import { getErrorMessage } from "../lib/errors";

const SETTINGS_RETRY_DELAYS_MS = [0, 16, 64] as const;
const TOOLBAR_PRESENTATION_ATTEMPTS = 3;
const COPY_SUCCESS_RESET_MS = 1_500;
const FIXED_ASK_ACTION_ID = "ask-ai";
const AI_CONFIGURATION_MESSAGES = [
  "请先为动作选择 AI 服务商",
  "请先为动作选择模型",
  "选择的服务商不存在",
  "选择的模型不存在或已被删除",
  "请先为服务商保存 API Key",
  "无法读取本地加密的 API Key",
] as const;

export function isAiConfigurationMessage(message: string): boolean {
  return AI_CONFIGURATION_MESSAGES.some((candidate) => message.includes(candidate));
}

export const AI_CONFIGURATION_SETTINGS_NOTICE =
  "请先配置 AI 服务商与模型，再为工具栏动作选择模型。";

export function settingsNoticeForConfigurationMessage(message: string): string {
  if (message.includes("API Key")) {
    return "请先为服务商保存 API Key，再为工具栏动作选择模型。";
  }
  if (message.includes("服务商不存在")) {
    return "所选服务商不可用，请重新配置 AI 服务商与模型。";
  }
  if (message.includes("模型不存在")) {
    return "所选模型不可用，请重新为工具栏动作选择模型。";
  }
  return AI_CONFIGURATION_SETTINGS_NOTICE;
}

function rendererErrorClass(error: unknown): string {
  if (error instanceof TypeError) return "TypeError";
  if (error instanceof RangeError) return "RangeError";
  if (error instanceof DOMException) return "DOMException";
  if (error instanceof Error) return "Error";
  if (error === null) return "null";
  return typeof error;
}

function logToolbarDiagnostic(
  stage: string,
  selectionId: string | undefined,
  attempt: number,
  error?: unknown,
): void {
  console.warn("[Popper][toolbar]", {
    stage: `${stage}:${attempt}`,
    ...(selectionId ? { selectionId } : {}),
    ...(arguments.length >= 4 ? { errorClass: rendererErrorClass(error) } : {}),
  });
}

function traceToolbarTiming(stage: string, startedAt: number | null): void {
  if (startedAt === null) return;
  try {
    if (window.localStorage.getItem("popper.selectionTrace") !== "1") return;
  } catch {
    return;
  }
  console.debug("[Popper][selection-timing]", {
    stage,
    durationMs: Math.max(0, performance.now() - startedAt),
  });
}

function toolbarControlIdFromElement(
  target: Element | null,
  toolbar: HTMLElement | null,
): string | null {
  const control = target?.closest<HTMLElement>("[data-toolbar-control]") ?? null;
  if (!control || !toolbar?.contains(control)) return null;
  if (control instanceof HTMLButtonElement && control.disabled) return null;
  if (control.getAttribute("aria-disabled") === "true") return null;
  return control.dataset.toolbarControl || null;
}

export function ToolbarApp(): JSX.Element {
  const toolbarRef = useRef<HTMLDivElement>(null);
  const askInputRef = useRef<HTMLTextAreaElement>(null);
  const askCursorRef = useRef<{ x: number; y: number } | undefined>(undefined);
  const askModeActivatedRef = useRef(false);
  const askNativeFrameExpandedRef = useRef(false);
  const askTransitionRef = useRef(false);
  const askExpandedRef = useRef(false);
  const focusClearFramesRef = useRef<number[]>([]);
  const copySuccessTimerRef = useRef<number | null>(null);
  const copySuccessActiveRef = useRef(false);
  const [settings, setSettings] = useState<PublicSettings | null>(null);
  const [selection, setSelection] = useState<SelectionPayload | null>(null);
  const [busyActionId, setBusyActionId] = useState<string | null>(null);
  const [copySuccessActionId, setCopySuccessActionId] = useState<string | null>(null);
  const [hoveredControlId, setHoveredControlId] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [askExpanded, setAskExpanded] = useState(false);
  const [askQuestion, setAskQuestion] = useState("");
  const operationGenerationRef = useRef(0);
  const selectionGenerationRef = useRef(0);
  const selectionTimingStartedAtRef = useRef<number | null>(null);

  const blurToolbarFocus = (): void => {
    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement && toolbarRef.current?.contains(activeElement)) {
      activeElement.blur();
    }
  };

  const clearRetainedToolbarFocus = (): void => {
    setHoveredControlId(null);
    focusClearFramesRef.current.forEach((frame) => cancelAnimationFrame(frame));
    focusClearFramesRef.current = [];
    blurToolbarFocus();

    const firstFrame = requestAnimationFrame(() => {
      blurToolbarFocus();
      const secondFrame = requestAnimationFrame(() => {
        blurToolbarFocus();
        focusClearFramesRef.current = [];
      });
      focusClearFramesRef.current = [secondFrame];
    });
    focusClearFramesRef.current = [firstFrame];
  };

  const cancelCopySuccessReset = (): void => {
    if (copySuccessTimerRef.current !== null) {
      window.clearTimeout(copySuccessTimerRef.current);
      copySuccessTimerRef.current = null;
    }
  };

  const resetCopySuccess = (): void => {
    copySuccessActiveRef.current = false;
    cancelCopySuccessReset();
    setCopySuccessActionId(null);
  };

  const confirmCopySuccess = (actionId: string): void => {
    cancelCopySuccessReset();
    copySuccessActiveRef.current = true;
    setCopySuccessActionId(actionId);
    copySuccessTimerRef.current = window.setTimeout(() => {
      copySuccessTimerRef.current = null;
      copySuccessActiveRef.current = false;
      setCopySuccessActionId((current) => (current === actionId ? null : current));
    }, COPY_SUCCESS_RESET_MS);
  };

  useEffect(() => {
    let disposed = false;
    let settingsRetryTimer = 0;

    const loadSettings = async (attempt: number): Promise<void> => {
      try {
        const value = await window._popper_.getSettings();
        if (!disposed) setSettings(value);
      } catch (error: unknown) {
        if (disposed) return;
        logToolbarDiagnostic("settings-read", undefined, attempt + 1, error);
        const nextDelay = SETTINGS_RETRY_DELAYS_MS[attempt + 1];
        if (nextDelay !== undefined) {
          settingsRetryTimer = window.setTimeout(() => void loadSettings(attempt + 1), nextDelay);
          return;
        }
        setMessage(getErrorMessage(error, "无法读取设置"));
      }
    };
    void loadSettings(0);

    const unsubscribeSelection = window._popper_.onSelection((nextSelection) => {
      selectionTimingStartedAtRef.current = performance.now();
      resetCopySuccess();
      clearRetainedToolbarFocus();
      selectionGenerationRef.current += 1;
      operationGenerationRef.current += 1;
      setSelection(nextSelection);
      setBusyActionId(null);
      setMessage("");
      setAskExpanded(false);
      setAskQuestion("");
    });
    const initialSelectionGeneration = selectionGenerationRef.current;
    const currentSelection = window._popper_.getCurrentSelection?.();
    void currentSelection
      ?.then((value) => {
        if (!disposed && value && selectionGenerationRef.current === initialSelectionGeneration) {
          clearRetainedToolbarFocus();
          setSelection(value);
        }
      })
      .catch((error: unknown) => {
        if (!disposed) logToolbarDiagnostic("selection-replay", undefined, 1, error);
      });
    const unsubscribeSettings = window._popper_.onSettingsChanged((nextSettings) => {
      resetCopySuccess();
      clearRetainedToolbarFocus();
      setSettings(nextSettings);
      if (!nextSettings.enabled) {
        runDetached(window._popper_.hideToolbar(), {
          scope: "toolbar",
          operation: "hide-disabled",
        });
      }
    });
    const unsubscribeToolbarPointer =
      window._popper_.onToolbarPointer?.((pointer) => {
        if (!pointer.inside) {
          setHoveredControlId(null);
          return;
        }

        const target = document.elementFromPoint(pointer.x, pointer.y);
        const nextControlId = toolbarControlIdFromElement(target, toolbarRef.current);
        setHoveredControlId((currentControlId) =>
          currentControlId === nextControlId ? currentControlId : nextControlId,
        );
      }) ?? (() => undefined);

    // Native dismiss force-hides the window; clear React selection / copy
    // success so a single outside click does not leave a stale toolbar shell.
    const unsubscribeDismiss =
      window._popper_.onToolbarDismissed?.((payload) => {
        resetCopySuccess();
        clearRetainedToolbarFocus();
        selectionGenerationRef.current += 1;
        operationGenerationRef.current += 1;
        setSelection((current) => {
          if (!payload.selectionId) return null;
          return current?.selectionId === payload.selectionId ? null : current;
        });
        setBusyActionId(null);
        setMessage("");
        setAskExpanded(false);
        setAskQuestion("");
      }) ?? (() => undefined);

    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        if (askExpandedRef.current) {
          event.preventDefault();
          setAskExpanded(false);
          setAskQuestion("");
          return;
        }
        resetCopySuccess();
        clearRetainedToolbarFocus();
        runDetached(window._popper_.hideToolbar(), {
          scope: "toolbar",
          operation: "hide-escape",
        });
      }
    };
    window.addEventListener("keydown", onKeyDown);

    return () => {
      disposed = true;
      window.clearTimeout(settingsRetryTimer);
      focusClearFramesRef.current.forEach((frame) => cancelAnimationFrame(frame));
      focusClearFramesRef.current = [];
      copySuccessActiveRef.current = false;
      cancelCopySuccessReset();
      unsubscribeSelection();
      unsubscribeSettings();
      unsubscribeToolbarPointer();
      unsubscribeDismiss();
      window.removeEventListener("keydown", onKeyDown);
    };
  }, []);

  const visibleActions = useMemo(
    () =>
      [...(settings?.actions ?? [])]
        .filter((action) => action.enabled && action.kind !== "ask")
        .sort((left, right) => left.order - right.order)
        .slice(0, MAX_ENABLED_ACTIONS),
    [settings],
  );

  useLayoutEffect(() => {
    askExpandedRef.current = askExpanded;
    const selectionId = selection?.selectionId;
    let disposed = false;
    if (!askExpanded) {
      if (askModeActivatedRef.current || askNativeFrameExpandedRef.current) {
        const shouldDeactivateInput = askModeActivatedRef.current;
        askModeActivatedRef.current = false;
        askNativeFrameExpandedRef.current = false;
        void (async () => {
          try {
            if (shouldDeactivateInput) {
              await window._popper_.setToolbarInputMode?.(false, selectionId);
            }
            if (disposed) return;
            const rectangle = toolbarRef.current?.getBoundingClientRect();
            if (rectangle) {
              await window._popper_.reportToolbarSize(
                {
                  width: Math.max(1, Math.ceil(rectangle.width)),
                  height: Math.max(1, Math.ceil(rectangle.height)),
                },
                selectionId,
              );
            }
          } catch (error) {
            if (!disposed) setMessage(getErrorMessage(error, "无法恢复工具栏尺寸"));
          }
        })();
      }
      return () => {
        disposed = true;
      };
    }

    void (async () => {
      let nativeInputModeActive = false;
      try {
        // React has committed the second-stage DOM before this layout effect.
        // Grow the native WebView first so a slow focusability/style transition
        // can never leave the composer clipped inside the compact toolbar frame.
        const rectangle = toolbarRef.current?.getBoundingClientRect();
        if (rectangle) {
          await window._popper_.reportToolbarSize(
            {
              width: Math.max(1, Math.ceil(rectangle.width)),
              height: Math.max(1, Math.ceil(rectangle.height)),
            },
            selectionId,
          );
          askNativeFrameExpandedRef.current = true;
        }
        if (disposed) return;
        let activated = false;
        let activationError: unknown;
        for (let attempt = 0; attempt < 3 && !disposed; attempt += 1) {
          try {
            activated = await (window._popper_.setToolbarInputMode?.(true, selectionId) ??
              Promise.resolve(true));
            activationError = undefined;
            break;
          } catch (error) {
            activationError = error;
            if (attempt < 2) {
              await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
            }
          }
        }
        if (activationError) throw activationError;
        if (activated === false) {
          throw new Error("当前选区已失效，请重新选择文字后重试");
        }
        nativeInputModeActive = true;
        if (disposed) {
          await window._popper_.setToolbarInputMode?.(false, selectionId);
          nativeInputModeActive = false;
          return;
        }
        askModeActivatedRef.current = true;
        let focused = false;
        let focusError: unknown;
        for (let attempt = 0; attempt < 3 && !disposed; attempt += 1) {
          try {
            focused = await (window._popper_.focusToolbarInput?.(selectionId) ??
              Promise.resolve(true));
            focusError = undefined;
            // Native focus can legitimately be rejected for one event-loop
            // turn while Windows removes WS_EX_NOACTIVATE. Only stop retrying
            // once the backend has verified that our HWND is foreground.
            if (focused) break;
            if (attempt < 2) {
              await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
            }
          } catch (error) {
            focusError = error;
            if (attempt < 2) {
              await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
            }
          }
        }
        if (focusError) throw focusError;
        if (focused === false) {
          throw new Error("当前选区已失效，无法聚焦输入框");
        }
        if (disposed) return;
        requestAnimationFrame(() => askInputRef.current?.focus());
      } catch (error) {
        // The composer is renderer state, not a native-focus transaction.
        // Windows can transiently reject foreground focus while the opening
        // click is settling; keep stage two visible instead of making the
        // interaction appear to do nothing.
        if (!disposed) {
          setMessage(getErrorMessage(error, "无法启用输入框"));
        }
      } finally {
        askTransitionRef.current = false;
      }
    })();
    return () => {
      disposed = true;
    };
  }, [askExpanded, selection?.selectionId]);

  useLayoutEffect(() => {
    const element = toolbarRef.current;
    const presentToolbar = window._popper_.presentToolbar;
    const selectionId = selection?.selectionId;
    if (!element || !presentToolbar || !selectionId || !settings) return;

    // Windows keeps the native frame hidden during staging. Commit the first
    // measured DOM size here so only the current renderer generation can make
    // the toolbar visible; the observer below handles later content changes.
    const rectangle = element.getBoundingClientRect();
    const size = {
      width: Math.max(1, Math.ceil(rectangle.width)),
      height: Math.max(1, Math.ceil(rectangle.height)),
    };
    let disposed = false;
    let retryFrame = 0;
    const timingStartedAt = selectionTimingStartedAtRef.current;
    traceToolbarTiming("renderer", timingStartedAt);
    const scheduleRetry = (attempt: number): void => {
      retryFrame = requestAnimationFrame(() => void present(attempt));
    };
    const currentSelectionStillMatches = async (attempt: number): Promise<boolean> => {
      const getCurrentSelection = window._popper_.getCurrentSelection;
      if (!getCurrentSelection) return true;
      try {
        const current = await getCurrentSelection();
        if (disposed) return false;
        if (current?.selectionId === selectionId) return true;
        if (current) {
          clearRetainedToolbarFocus();
          selectionGenerationRef.current += 1;
          operationGenerationRef.current += 1;
          setSelection(current);
          setBusyActionId(null);
          setMessage("");
        }
        return false;
      } catch (error: unknown) {
        if (!disposed) {
          logToolbarDiagnostic("selection-check", selectionId, attempt + 1, error);
        }
        // The renderer still owns this React selection. A failed replay query
        // is not evidence that the native selection disappeared.
        return true;
      }
    };
    const recover = async (attempt: number): Promise<void> => {
      const recoverToolbar = window._popper_.recoverToolbar;
      if (!recoverToolbar || disposed) return;
      logToolbarDiagnostic("presentation-rebuild", selectionId, attempt + 1);
      try {
        await recoverToolbar(selectionId);
      } catch (error: unknown) {
        // Destroying the calling WebView can close its IPC response channel.
        // The native Destroyed handler still recreates the singleton window.
        if (!disposed) {
          logToolbarDiagnostic("presentation-rebuild-ipc", selectionId, attempt + 1, error);
        }
      }
    };
    const present = async (attempt: number): Promise<void> => {
      let presented = false;
      try {
        presented = await presentToolbar(selectionId, size);
      } catch (error: unknown) {
        if (!disposed) {
          logToolbarDiagnostic("presentation-ipc", selectionId, attempt + 1, error);
        }
      }

      if (disposed) return;
      if (presented) {
        traceToolbarTiming("native-visible", timingStartedAt);
        selectionTimingStartedAtRef.current = null;
        return;
      }
      const stillMatches = await currentSelectionStillMatches(attempt);
      if (disposed || !stillMatches) return;
      if (attempt + 1 < TOOLBAR_PRESENTATION_ATTEMPTS) {
        scheduleRetry(attempt + 1);
        return;
      }
      await recover(attempt);
    };
    void present(0);
    return () => {
      disposed = true;
      cancelAnimationFrame(retryFrame);
    };
  }, [selection?.selectionId, settings, visibleActions]);

  useEffect(() => {
    const element = toolbarRef.current;
    if (!element) return;
    const selectionId = selection?.selectionId;

    let frame = 0;
    const initialRectangle = element.getBoundingClientRect();
    // `presentToolbar` already commits this initial size. Seed the observer
    // from the same layout so it only reports later, real DOM changes instead
    // of issuing a redundant native resize IPC on every selection.
    let lastWidth = Math.max(1, Math.ceil(initialRectangle.width));
    let lastHeight = Math.max(1, Math.ceil(initialRectangle.height));
    const report = (): void => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const rectangle = element.getBoundingClientRect();
        const width = Math.max(1, Math.ceil(rectangle.width));
        const height = Math.max(1, Math.ceil(rectangle.height));
        if (width === lastWidth && height === lastHeight) return;
        lastWidth = width;
        lastHeight = height;
        runDetached(window._popper_.reportToolbarSize({ width, height }, selectionId), {
          scope: "toolbar",
          operation: "report-size",
        });
      });
    };

    const observer = new ResizeObserver(report);
    observer.observe(element);

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
    // Keep one observer alive for the lifetime of the current selection. If it
    // is recreated after `askExpanded` changes, the expanded rectangle becomes
    // its baseline and the native WebView never receives the resize that makes
    // the second-stage composer visible.
  }, [selection?.selectionId]);

  const runAction = async (
    actionId: string,
    event?: MouseEvent<HTMLButtonElement>,
    initialQuestion?: string,
  ): Promise<void> => {
    if (busyActionId) return;
    const action = visibleActions.find((candidate) => candidate.id === actionId);
    const actionKind = actionId === FIXED_ASK_ACTION_ID ? "ask" : action?.kind;
    if (!actionKind) return;
    resetCopySuccess();
    const pointerTriggered = (event?.detail ?? 0) > 0;
    const actedSelectionId = selection?.selectionId;
    const cursor = initialQuestion
      ? askCursorRef.current
      : pointerTriggered &&
          event &&
          Number.isFinite(event.screenX) &&
          Number.isFinite(event.screenY)
        ? { x: event.screenX, y: event.screenY }
        : undefined;
    if (pointerTriggered) {
      clearRetainedToolbarFocus();
    }
    setBusyActionId(actionId);
    setMessage("");
    const operationGeneration = operationGenerationRef.current + 1;
    operationGenerationRef.current = operationGeneration;

    try {
      const result = initialQuestion
        ? await window._popper_.runAction(
            actionId,
            cursor,
            actedSelectionId,
            undefined,
            initialQuestion,
          )
        : await window._popper_.runAction(actionId, cursor, actedSelectionId);
      if (operationGenerationRef.current !== operationGeneration) return;
      if (!result.accepted) {
        if (isAiConfigurationMessage(result.message) && window._popper_.openSettings) {
          const notice = settingsNoticeForConfigurationMessage(result.message);
          try {
            await window._popper_.openSettings({
              focus: "actions",
              notice,
            });
            setSelection((current) => (current?.selectionId === actedSelectionId ? null : current));
            setAskExpanded(false);
            setAskQuestion("");
            await window._popper_.hideToolbar(actedSelectionId);
          } catch (error) {
            setMessage(getErrorMessage(error, result.message));
          }
          return;
        }
        setMessage(result.message);
        return;
      }
      if (actionKind === "copy" && action) {
        confirmCopySuccess(action.id);
        return;
      }
      // Retire the consumed selection before clearing the busy state. Without
      // this, the layout effect can present the old toolbar again while the
      // backend's native hide is still being committed on Windows.
      setSelection((current) => (current?.selectionId === actedSelectionId ? null : current));
      setAskExpanded(false);
      setAskQuestion("");
      await window._popper_.hideToolbar(actedSelectionId);
    } catch (error) {
      if (operationGenerationRef.current === operationGeneration) {
        setMessage(getErrorMessage(error));
      }
    } finally {
      if (operationGenerationRef.current === operationGeneration) {
        setBusyActionId(null);
      }
    }
  };

  const openAsk = (event: MouseEvent<HTMLButtonElement>): void => {
    if (busyActionId || askTransitionRef.current || askExpanded) return;
    const selectionId = selection?.selectionId;
    if (!selectionId) return;
    askCursorRef.current =
      event.detail > 0 && Number.isFinite(event.screenX) && Number.isFinite(event.screenY)
        ? { x: event.screenX, y: event.screenY }
        : undefined;
    resetCopySuccess();
    setMessage("");
    askTransitionRef.current = true;
    askExpandedRef.current = true;
    // The layout effect performs native resize -> input mode -> focus after
    // React has committed the expanded composer.
    setAskExpanded(true);
  };

  const closeAsk = async (): Promise<void> => {
    const actedSelectionId = selection?.selectionId;
    if (!actedSelectionId) return;

    try {
      // Keep the expanded renderer mounted until the native window is hidden.
      // Collapsing first exposes the compact toolbar for at least one frame,
      // which makes a close action behave like a back action on Windows.
      await window._popper_.hideToolbar(actedSelectionId);
      selectionGenerationRef.current += 1;
      operationGenerationRef.current += 1;
      resetCopySuccess();
      clearRetainedToolbarFocus();
      setSelection((current) => (current?.selectionId === actedSelectionId ? null : current));
      setBusyActionId(null);
      setMessage("");
      setAskExpanded(false);
      setAskQuestion("");
    } catch (error) {
      setMessage(getErrorMessage(error, "无法关闭问 AI 窗口"));
    }
  };

  const submitAsk = (): void => {
    const question = askQuestion.trim();
    if (!question || busyActionId) return;
    void runAction(FIXED_ASK_ACTION_ID, undefined, question);
  };

  const openSettings = async (notice?: string): Promise<void> => {
    if (!window._popper_.openSettings) return;
    const actedSelectionId = selection?.selectionId;
    try {
      await window._popper_.openSettings({
        focus: "actions",
        ...(notice ? { notice } : {}),
      });
      setSelection((current) => (current?.selectionId === actedSelectionId ? null : current));
      setAskExpanded(false);
      setAskQuestion("");
      await window._popper_.hideToolbar(actedSelectionId);
    } catch (error) {
      setMessage(getErrorMessage(error, "无法打开设置"));
    }
  };

  const iconOnly = settings?.toolbar.displayMode === "icon-only";

  const updateHoveredControl = (target: EventTarget | null): void => {
    const nextControlId = toolbarControlIdFromElement(
      target instanceof Element ? target : null,
      toolbarRef.current,
    );
    setHoveredControlId((currentControlId) =>
      currentControlId === nextControlId ? currentControlId : nextControlId,
    );
  };

  return (
    <div
      ref={toolbarRef}
      className="toolbar-shell"
      role="toolbar"
      aria-label={selection ? `处理来自 ${selection.sourceApp.name} 的选中文本` : "划词动作"}
      onPointerMoveCapture={(event) => updateHoveredControl(event.target)}
      onPointerLeave={() => setHoveredControlId(null)}
      onMouseMoveCapture={(event) => updateHoveredControl(event.target)}
      onMouseLeave={() => setHoveredControlId(null)}
    >
      <div className={`toolbar-pill ${askExpanded ? "toolbar-pill--ask" : ""}`}>
        {askExpanded ? (
          <div className="toolbar-ask">
            <div className="toolbar-ask__header">
              <span className="toolbar-ask__selection" title={selection?.text}>
                {selection?.text}
              </span>
              <button
                type="button"
                className="toolbar-ask__close"
                data-toolbar-control="ask-close"
                aria-label="关闭问 AI 窗口"
                onClick={() => void closeAsk()}
              >
                <X size={14} />
              </button>
            </div>
            <div className="toolbar-ask__composer">
              <span className="toolbar-ask__model" aria-hidden="true">
                <span className="toolbar-ai-mark">
                  <Sparkles size={13} />
                </span>
                <ChevronDown size={10} />
              </span>
              <textarea
                ref={askInputRef}
                aria-label="向 AI 提问"
                placeholder="向 AI 询问上述内容"
                rows={1}
                maxLength={20_000}
                value={askQuestion}
                disabled={busyActionId !== null}
                onChange={(event) => setAskQuestion(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    submitAsk();
                  }
                }}
              />
              <button
                type="button"
                className="toolbar-ask__send"
                data-toolbar-control="ask-send"
                aria-label="发送问题"
                disabled={!askQuestion.trim() || busyActionId !== null}
                onClick={submitAsk}
              >
                {busyActionId ? (
                  <LoaderCircle className="spin" size={16} />
                ) : (
                  <SendHorizontal size={17} />
                )}
              </button>
            </div>
          </div>
        ) : (
          <>
            <button
              className="toolbar-action toolbar-action--icon-only toolbar-action--fixed-ask"
              type="button"
              data-toolbar-control="fixed-ask"
              data-hovered={hoveredControlId === "fixed-ask" ? "true" : undefined}
              title="问 AI"
              aria-label="问 AI"
              onMouseDown={(event) => event.preventDefault()}
              onClick={(event) => void openAsk(event)}
            >
              <span className="toolbar-ai-mark">
                <Sparkles size={13} />
              </span>
            </button>
            {visibleActions.map((action) => {
              const busy = busyActionId === action.id;
              const muted = busyActionId !== null && !busy;
              const copySucceeded = action.kind === "copy" && copySuccessActionId === action.id;
              return (
                <button
                  className={`toolbar-action ${iconOnly ? "toolbar-action--icon-only" : ""} ${muted ? "toolbar-action--muted" : ""}`}
                  type="button"
                  key={action.id}
                  data-toolbar-control={`action:${action.id}`}
                  data-hovered={hoveredControlId === `action:${action.id}` ? "true" : undefined}
                  title={action.name}
                  aria-label={action.name}
                  disabled={busy}
                  aria-disabled={muted ? "true" : undefined}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={(event) => void runAction(action.id, event)}
                >
                  {busy ? (
                    <LoaderCircle className="spin" size={16} aria-hidden="true" />
                  ) : (
                    <ActionIcon
                      className={copySucceeded ? "toolbar-copy-success" : undefined}
                      name={copySucceeded ? "clipboard-check" : action.icon}
                      size={16}
                      strokeWidth={2}
                    />
                  )}
                  {!iconOnly && <span>{action.name}</span>}
                </button>
              );
            })}
          </>
        )}
      </div>
      {message && (
        <div
          className={`toolbar-message ${isAiConfigurationMessage(message) ? "toolbar-message--configuration" : ""}`}
          role="alert"
        >
          <CircleAlert className="toolbar-message__icon" size={16} aria-hidden="true" />
          <span className="toolbar-message__content">
            {isAiConfigurationMessage(message) && <strong>AI 功能尚未配置</strong>}
            <span>{message}</span>
          </span>
          {isAiConfigurationMessage(message) && window._popper_.openSettings && (
            <button
              className="toolbar-message__action"
              type="button"
              data-toolbar-control="open-settings"
              data-hovered={hoveredControlId === "open-settings" ? "true" : undefined}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() =>
                void openSettings(
                  isAiConfigurationMessage(message)
                    ? settingsNoticeForConfigurationMessage(message)
                    : undefined,
                )
              }
            >
              <Settings2 size={14} aria-hidden="true" />
              <span>打开设置</span>
            </button>
          )}
        </div>
      )}
    </div>
  );
}
