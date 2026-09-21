import {
  DEFAULT_PUBLIC_SETTINGS,
  TAURI_EVENTS,
  type ResultReadyAck,
  type ResultSessionSnapshot,
} from "../../shared";
import { installTauriBridge } from "./tauriBridge";

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

describe("Tauri renderer bridge", () => {
  it("exports the live selection event name used by the bridge", () => {
    expect(TAURI_EVENTS.selection).toBe("popper:selection");
    expect(TAURI_EVENTS.actionStream).toBe("popper:action-stream");
    expect(TAURI_EVENTS.toolbarDismissed).toBe("popper:toolbar-dismissed");
    expect(TAURI_EVENTS.resultSelectionShortcut).toBe("popper:result-selection-shortcut");
  });

  it("waits for the stream listener and forwards command arguments in Tauri form", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const consoleWarn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    let markActionListenerReady: ((unlisten: () => void) => void) | undefined;
    let forwardActionEvent: ((event: { payload: unknown }) => void) | undefined;
    let forwardToolbarPointer: ((event: { payload: unknown }) => void) | undefined;
    let forwardToolbarDismissed: ((event: { payload: unknown }) => void) | undefined;
    let forwardResultSelectionShortcut: ((event: { payload: unknown }) => void) | undefined;
    let forwardSettingsCloseRequest: ((event: { payload: unknown }) => void) | undefined;
    let selectionListenAttempts = 0;
    listenMock.mockImplementation(
      (eventName: string, listener: (event: { payload: unknown }) => void) => {
        if (eventName === "popper:selection") {
          selectionListenAttempts += 1;
          if (selectionListenAttempts < 3) {
            return Promise.reject(new Error("sensitive transient listener failure"));
          }
        }
        if (eventName === "popper:action-stream") {
          forwardActionEvent = listener;
          return new Promise<() => void>((resolve) => {
            markActionListenerReady = resolve;
          });
        }
        if (eventName === "popper:toolbar-pointer") {
          forwardToolbarPointer = listener;
        }
        if (eventName === "popper:toolbar-dismissed") {
          forwardToolbarDismissed = listener;
        }
        if (eventName === "popper:result-selection-shortcut") {
          forwardResultSelectionShortcut = listener;
        }
        if (eventName === "popper:settings-close-requested") {
          forwardSettingsCloseRequest = listener;
        }
        return Promise.resolve(() => undefined);
      },
    );
    const readySnapshot: ResultSessionSnapshot = {
      sessionId: "session-1",
      sessionGeneration: 7,
      requestId: "request-2",
      requestGeneration: 2,
      actionId: "translate",
      providerId: "openai-compatible",
      modelId: "model-1",
      selection: {
        selectionId: "selection-1",
        text: "hello",
        sourceApp: { name: "Test", bundleId: null },
        anchor: { kind: "cursor", x: 10, y: 20 },
        direction: "unknown",
        isFullscreen: false,
      },
      status: "streaming",
      content: "😀",
      thinkingContent: "",
      contentScalarCount: 1,
      lastContentSequence: 10,
      lastSequence: 11,
      handshakeGeneration: 4,
      errorMessage: "",
      retryable: false,
      pinned: false,
    };

    invokeMock.mockImplementation((command: string) => {
      if (command === "toolbar_ready") return Promise.resolve(null);
      if (command === "recover_toolbar") return Promise.resolve(true);
      if (command === "begin_result_ready") {
        return Promise.resolve(readySnapshot);
      }
      if (command === "ack_result_ready") return Promise.resolve(true);
      if (command === "retry_action") {
        return Promise.resolve({
          accepted: true,
          sessionId: "session-1",
          requestId: "request-2",
        });
      }
      if (command === "continue_action") {
        return Promise.resolve({
          accepted: true,
          sessionId: "session-1",
          requestId: "request-3",
        });
      }
      if (command === "list_provider_models") {
        return Promise.resolve({
          ok: true,
          models: [
            {
              id: "model-a",
              name: "Model A",
              thinkingLevels: ["low"],
            },
            {
              id: "model-b",
              name: "Model B",
              thinkingLevels: [],
            },
          ],
        });
      }
      if (command === "sync_provider_models") {
        return Promise.resolve({
          ok: true,
          models: [
            {
              id: "model-1",
              name: "Model 1",
              thinkingLevels: ["low", "medium", "high"],
            },
          ],
          settings: DEFAULT_PUBLIC_SETTINGS,
        });
      }
      if (command === "reset_result_size") return Promise.resolve(DEFAULT_PUBLIC_SETTINGS);
      if (command === "get_accessibility_status") {
        return Promise.resolve({
          platform: "windows",
          trusted: true,
          canRequest: false,
          available: false,
          diagnostics: {
            selectionMonitorError: "无法启动系统划词监听。",
            shortcutError: "全局快捷键注册失败。",
          },
        });
      }
      return Promise.resolve(undefined);
    });

    installTauriBridge();

    await expect(window._popper_.getCurrentSelection?.()).resolves.toBeNull();
    expect(selectionListenAttempts).toBe(3);
    expect(JSON.stringify(consoleWarn.mock.calls)).not.toContain(
      "sensitive transient listener failure",
    );

    await expect(window._popper_.recoverToolbar?.("selection-1")).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("recover_toolbar", { selectionId: "selection-1" });

    await window._popper_.setToolbarInputMode?.(true, "selection-1");
    expect(invokeMock).toHaveBeenCalledWith("set_toolbar_input_mode", {
      active: true,
      selectionId: "selection-1",
    });

    await window._popper_.focusToolbarInput?.("selection-1");
    expect(invokeMock).toHaveBeenCalledWith("focus_toolbar_input", {
      selectionId: "selection-1",
    });

    const toolbarPointerListener = vi.fn();
    const unsubscribeToolbarPointer = window._popper_.onToolbarPointer?.(toolbarPointerListener);
    expect(listenMock).toHaveBeenCalledWith("popper:toolbar-pointer", expect.any(Function));
    forwardToolbarPointer?.({ payload: { x: 24.5, y: 18, inside: true } });
    expect(toolbarPointerListener).toHaveBeenLastCalledWith({ x: 24.5, y: 18, inside: true });

    // Invalid or expanded native payloads are rejected at the renderer
    // boundary instead of leaking into DOM hit-testing.
    forwardToolbarPointer?.({ payload: { x: 24.5, y: 18, inside: true, control: "copy" } });
    expect(toolbarPointerListener).toHaveBeenCalledTimes(1);
    unsubscribeToolbarPointer?.();
    forwardToolbarPointer?.({ payload: { x: 80, y: 18, inside: true } });
    expect(toolbarPointerListener).toHaveBeenCalledTimes(1);

    const unsubscribeFailing = window._popper_.onToolbarPointer?.(() => {
      throw new Error("listener failure");
    });
    const survivingListener = vi.fn();
    const unsubscribeSurviving = window._popper_.onToolbarPointer?.(survivingListener);
    forwardToolbarPointer?.({ payload: { x: 96, y: 18, inside: true } });
    expect(survivingListener).toHaveBeenCalledWith({ x: 96, y: 18, inside: true });
    expect(consoleError).toHaveBeenCalled();
    unsubscribeFailing?.();
    unsubscribeSurviving?.();

    const toolbarDismissedListener = vi.fn();
    const unsubscribeToolbarDismissed =
      window._popper_.onToolbarDismissed?.(toolbarDismissedListener);
    expect(listenMock).toHaveBeenCalledWith("popper:toolbar-dismissed", expect.any(Function));
    forwardToolbarDismissed?.({
      payload: { selectionId: "selection-1", reason: "mouseDown" },
    });
    expect(toolbarDismissedListener).toHaveBeenLastCalledWith({
      selectionId: "selection-1",
      reason: "mouseDown",
    });
    // Missing selectionId is valid (dismiss after selection already cleared).
    forwardToolbarDismissed?.({ payload: { reason: "escape" } });
    expect(toolbarDismissedListener).toHaveBeenLastCalledWith({ reason: "escape" });
    // Invalid payloads are rejected at the renderer boundary.
    forwardToolbarDismissed?.({ payload: { selectionId: "selection-1" } });
    expect(toolbarDismissedListener).toHaveBeenCalledTimes(2);
    unsubscribeToolbarDismissed?.();
    forwardToolbarDismissed?.({ payload: { reason: "mouseDown" } });
    expect(toolbarDismissedListener).toHaveBeenCalledTimes(2);

    const resultSelectionShortcutListener = vi.fn();
    const unsubscribeResultSelectionShortcut = window._popper_.onResultSelectionShortcut?.(
      resultSelectionShortcutListener,
    );
    forwardResultSelectionShortcut?.({ payload: null });
    expect(resultSelectionShortcutListener).toHaveBeenCalledTimes(1);
    unsubscribeResultSelectionShortcut?.();

    const settingsCloseListener = vi.fn();
    const unsubscribeSettingsClose =
      window._popper_.onSettingsCloseRequested?.(settingsCloseListener);
    expect(listenMock).toHaveBeenCalledWith(
      "popper:settings-close-requested",
      expect.any(Function),
    );
    forwardSettingsCloseRequest?.({ payload: null });
    expect(settingsCloseListener).toHaveBeenCalledTimes(1);
    unsubscribeSettingsClose?.();

    // beginResultReady waits for the action-stream listener before invoke so
    // the native ready flush cannot race listener installation.
    const beginPromise = window._popper_.beginResultReady("session-1");
    await Promise.resolve();
    expect(invokeMock).not.toHaveBeenCalledWith("begin_result_ready", expect.anything());

    expect(markActionListenerReady).toBeTypeOf("function");
    markActionListenerReady?.(() => undefined);
    await expect(beginPromise).resolves.toEqual(readySnapshot);
    expect(invokeMock).toHaveBeenCalledWith("begin_result_ready", { sessionId: "session-1" });
    const ack: ResultReadyAck = {
      sessionId: "session-1",
      sessionGeneration: 7,
      requestGeneration: 2,
      lastSequence: 11,
      handshakeGeneration: 4,
    };
    await expect(window._popper_.ackResultReady(ack)).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("ack_result_ready", { ack });
    expect(invokeMock).not.toHaveBeenCalledWith("ack_result_ready", {
      sessionId: "session-1",
      ack,
    });

    const actionListener = vi.fn();
    const unsubscribeAction = window._popper_.onActionEvent(actionListener);
    forwardActionEvent?.({
      payload: {
        sessionId: "session-1",
        sessionGeneration: 7,
        requestId: "request-2",
        requestGeneration: 2,
        actionId: "translate",
        type: "delta",
        delta: "missing sequence",
      },
    });
    expect(actionListener).not.toHaveBeenCalled();
    const orderedEvent = {
      sessionId: "session-1",
      sessionGeneration: 7,
      requestId: "request-2",
      requestGeneration: 2,
      sequence: 12,
      actionId: "translate",
      type: "delta" as const,
      delta: " ordered",
    };
    forwardActionEvent?.({ payload: orderedEvent });
    expect(actionListener).toHaveBeenCalledWith(orderedEvent);
    unsubscribeAction();

    await window._popper_.recordResultRendererMarker("session-1", "request-2", "firstDomCommit");
    expect(invokeMock).toHaveBeenCalledWith("record_result_renderer_marker", {
      sessionId: "session-1",
      requestId: "request-2",
      marker: "firstDomCommit",
    });
    const markerCall = invokeMock.mock.calls.find(
      ([command]) => command === "record_result_renderer_marker",
    );
    expect(markerCall?.[1]).toEqual({
      sessionId: "session-1",
      requestId: "request-2",
      marker: "firstDomCommit",
    });
    expect(Object.keys(markerCall?.[1] as Record<string, unknown>).sort()).toEqual([
      "marker",
      "requestId",
      "sessionId",
    ]);
    await expect(
      window._popper_.recordResultRendererMarker(
        "session-1",
        "request-2",
        "firstDelta" as "firstDomCommit",
      ),
    ).rejects.toThrow();

    await window._popper_.prepareResultReveal?.("session-1");
    expect(invokeMock).toHaveBeenCalledWith("prepare_result_reveal", { sessionId: "session-1" });
    await window._popper_.commitResultReveal?.("session-1");
    expect(invokeMock).toHaveBeenCalledWith("commit_result_reveal", { sessionId: "session-1" });
    await window._popper_.failResultReveal?.("session-1", "renderer failed");
    expect(invokeMock).toHaveBeenCalledWith("fail_result_reveal", {
      sessionId: "session-1",
      message: "renderer failed",
    });

    await window._popper_.runAction("search", undefined, "selection-1");
    expect(invokeMock).toHaveBeenCalledWith("run_action", {
      actionId: "search",
      cursor: null,
      selectionId: "selection-1",
      searchEngineId: null,
      initialQuestion: null,
    });
    await window._popper_.runAction("search", undefined, "selection-1", "bing-china");
    expect(invokeMock).toHaveBeenCalledWith("run_action", {
      actionId: "search",
      cursor: null,
      selectionId: "selection-1",
      searchEngineId: "bing-china",
      initialQuestion: null,
    });

    await expect(
      window._popper_.retryAction("session-1", {
        targetLanguage: "en-US",
        providerId: "provider-1",
        modelId: "model-2",
      }),
    ).resolves.toEqual({
      accepted: true,
      sessionId: "session-1",
      requestId: "request-2",
    });
    expect(invokeMock).toHaveBeenCalledWith("retry_action", {
      sessionId: "session-1",
      targetLanguage: "en-US",
      providerId: "provider-1",
      modelId: "model-2",
    });

    await expect(window._popper_.continueAction?.("session-1", "继续解释")).resolves.toEqual({
      accepted: true,
      sessionId: "session-1",
      requestId: "request-3",
    });
    expect(invokeMock).toHaveBeenCalledWith("continue_action", {
      sessionId: "session-1",
      question: "继续解释",
    });

    await expect(window._popper_.listProviderModels?.("provider-1")).resolves.toEqual({
      ok: true,
      models: [
        {
          id: "model-a",
          name: "Model A",
          thinkingLevels: ["low"],
        },
        {
          id: "model-b",
          name: "Model B",
          thinkingLevels: [],
        },
      ],
    });
    expect(invokeMock).toHaveBeenCalledWith("list_provider_models", { providerId: "provider-1" });

    await expect(window._popper_.syncProviderModels?.("provider-1")).resolves.toEqual({
      ok: true,
      models: [
        {
          id: "model-1",
          name: "Model 1",
          thinkingLevels: ["low", "medium", "high"],
        },
      ],
      settings: DEFAULT_PUBLIC_SETTINGS,
    });
    expect(invokeMock).toHaveBeenCalledWith("sync_provider_models", { providerId: "provider-1" });

    await window._popper_.setResultPinned?.("session-1", true);
    expect(invokeMock).toHaveBeenCalledWith("set_result_pinned", {
      sessionId: "session-1",
      pinned: true,
    });

    await window._popper_.showResultSelection?.("session-1", "selected result", { x: 120, y: 240 });
    expect(invokeMock).toHaveBeenCalledWith("show_result_selection", {
      sessionId: "session-1",
      text: "selected result",
      cursor: { x: 120, y: 240 },
      forceCapture: false,
    });

    await window._popper_.hideResultSelection?.("session-1");
    expect(invokeMock).toHaveBeenCalledWith("hide_result_selection", {
      sessionId: "session-1",
    });

    await window._popper_.resetResultSize?.();
    expect(invokeMock).toHaveBeenCalledWith("reset_result_size");

    await window._popper_.settingsReady?.();
    expect(invokeMock).toHaveBeenCalledWith("settings_ready");

    await window._popper_.openSettings?.();
    expect(invokeMock).toHaveBeenCalledWith("open_settings", { options: null });

    await window._popper_.openSettings?.({ focus: "actions", notice: "请先配置" });
    expect(invokeMock).toHaveBeenCalledWith("open_settings", {
      options: { focus: "actions", notice: "请先配置" },
    });

    invokeMock.mockResolvedValueOnce({ focus: "actions", notice: "提示" });
    await expect(window._popper_.takeSettingsGuidance?.()).resolves.toEqual({
      focus: "actions",
      notice: "提示",
    });
    expect(invokeMock).toHaveBeenCalledWith("take_settings_guidance");

    await window._popper_.quitApp?.();
    expect(invokeMock).toHaveBeenCalledWith("quit_app");

    await expect(window._popper_.getAccessibilityStatus()).resolves.toEqual({
      platform: "windows",
      trusted: true,
      canRequest: false,
      available: false,
      diagnostics: {
        selectionMonitorError: "无法启动系统划词监听。",
        shortcutError: "全局快捷键注册失败。",
      },
    });
    consoleError.mockRestore();
    consoleWarn.mockRestore();
  });

  it("defaults runtime availability safely when hot reloading against the 0.3.5 backend", async () => {
    listenMock.mockResolvedValue(() => undefined);
    invokeMock.mockResolvedValue({ platform: "windows", trusted: true, canRequest: false });

    installTauriBridge();

    await expect(window._popper_.getAccessibilityStatus()).resolves.toEqual({
      platform: "windows",
      trusted: true,
      canRequest: false,
      available: true,
      diagnostics: {},
    });
  });

  it("parses listProviderModels error payload without writing settings", async () => {
    listenMock.mockResolvedValue(() => undefined);
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_provider_models") {
        return Promise.resolve({
          ok: false,
          message: "无法连接服务商",
          status: 401,
        });
      }
      return Promise.resolve(undefined);
    });

    installTauriBridge();

    await expect(window._popper_.listProviderModels?.("provider-1")).resolves.toEqual({
      ok: false,
      message: "无法连接服务商",
      status: 401,
    });
    expect(invokeMock).toHaveBeenCalledWith("list_provider_models", { providerId: "provider-1" });
  });
});
