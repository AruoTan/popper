import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";

import {
  DEFAULT_ACTION_PROMPTS,
  DEFAULT_PROVIDER_ID,
  DEFAULT_PUBLIC_SETTINGS,
  type PublicSettings,
  type SettingsGuidance,
  type SettingsUpdate,
  type WindowPopperApi,
} from "../../shared";
import { SettingsApp } from "./SettingsApp";
import { settingsGuidanceInbox } from "./settingsGuidanceInbox";

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function stubScrollIntoView(): ReturnType<typeof vi.fn> {
  const scrollIntoView = vi.fn();
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: scrollIntoView,
  });
  return scrollIntoView;
}

async function goToSettingsSection(label: string): Promise<void> {
  await screen.findByRole("navigation", { name: "设置分区" });
  fireEvent.click(screen.getByRole("button", { name: label }));
}

async function openProviderConfigurations(): Promise<void> {
  await goToSettingsSection("通用");
  for (const toggle of screen.getAllByRole("button", { name: /^展开.*配置$/ })) {
    fireEvent.click(toggle);
  }
}

function withoutDefaultProvider(): PublicSettings {
  return {
    ...DEFAULT_PUBLIC_SETTINGS,
    providers: [],
    actions: DEFAULT_PUBLIC_SETTINGS.actions.map((action) =>
      "providerId" in action ? { ...action, providerId: "", modelId: "" } : action,
    ),
  };
}

function installDefaultBridge(overrides: Partial<WindowPopperApi> = {}): void {
  window._popper_ = {
    getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
    updateSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
    getAccessibilityStatus: vi.fn(async () => ({
      platform: "windows",
      trusted: true,
      canRequest: false,
    })),
    requestAccessibility: vi.fn(),
    getSelectionDetectionDebugEnabled: vi.fn(async () => false),
    setSelectionDetectionDebugEnabled: vi.fn(async (enabled: boolean) => enabled),
    onSettingsChanged: vi.fn(() => () => undefined),
    ...overrides,
  } as unknown as WindowPopperApi;
}

describe("SettingsApp provider deletion", () => {
  it("toggles runtime diagnostics immediately without saving and keeps their state on reopening", async () => {
    let runtimeEnabled = false;
    const update = vi.fn(async (enabled: boolean) => { runtimeEnabled = enabled; return enabled; });
    const save = vi.fn(async () => DEFAULT_PUBLIC_SETTINGS);
    installDefaultBridge({
      getSelectionDetectionDebugEnabled: vi.fn(async () => runtimeEnabled),
      setSelectionDetectionDebugEnabled: update,
      updateSettings: save,
    });
    const view = render(<SettingsApp />);
    const toggle = await screen.findByRole("switch", { name: "DEV Debug" });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).not.toBeChecked();
    expect(toggle.closest(".settings-shell__nav")).toBeInTheDocument();
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toBeChecked());
    expect(update).toHaveBeenCalledWith(true);
    expect(save).not.toHaveBeenCalled();
    expect(screen.getByText("所有更改均已保存")).toBeInTheDocument();

    view.unmount();
    const reopened = render(<SettingsApp />);
    await waitFor(() => expect(screen.getByRole("switch", { name: "DEV Debug" })).toBeChecked());
    fireEvent.click(screen.getByRole("switch", { name: "DEV Debug" }));
    await waitFor(() => expect(screen.getByRole("switch", { name: "DEV Debug" })).not.toBeChecked());
    expect(update).toHaveBeenLastCalledWith(false);
    reopened.unmount();

    // A new native process starts with the runtime state reset.
    runtimeEnabled = false;
    render(<SettingsApp />);
    const restartedToggle = await screen.findByRole("switch", { name: "DEV Debug" });
    await waitFor(() => expect(restartedToggle).toBeEnabled());
    expect(restartedToggle).not.toBeChecked();
    expect(save).not.toHaveBeenCalled();
  });

  it("keeps the previous debug state and shows a toggle failure", async () => {
    installDefaultBridge({ setSelectionDetectionDebugEnabled: vi.fn().mockRejectedValue(new Error("failed")) });
    render(<SettingsApp />);
    const toggle = await screen.findByRole("switch", { name: "DEV Debug" });
    await waitFor(() => expect(toggle).toBeEnabled());
    fireEvent.click(toggle);
    expect(await screen.findByRole("alert")).toHaveTextContent("切换 DEV Debug 失败");
    expect(toggle).not.toBeChecked();
    expect(toggle).toBeEnabled();
  });

  it("does not show Windows selection debugging on macOS", async () => {
    installDefaultBridge({ getAccessibilityStatus: vi.fn(async () => ({ platform: "darwin" as const, trusted: true, canRequest: false, available: true, diagnostics: {} })) });
    render(<SettingsApp />);
    await screen.findByRole("heading", { name: "辅助功能权限" });
    expect(screen.queryByRole("switch", { name: "DEV Debug" })).not.toBeInTheDocument();
  });

  it("retains guidance across StrictMode cleanup and applies it once after remount", async () => {
    const pendingGuidance = deferred<SettingsGuidance | null>();
    const takeSettingsGuidance = vi.fn(() => pendingGuidance.promise);
    const scrollIntoView = stubScrollIntoView();
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "windows",
        trusted: true,
        canRequest: false,
      })),
      requestAccessibility: vi.fn(),
      takeSettingsGuidance,
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(
      <StrictMode>
        <SettingsApp />
      </StrictMode>,
    );
    expect(takeSettingsGuidance).toHaveBeenCalledOnce();

    act(() => pendingGuidance.resolve({ focus: "providers", notice: "请配置服务商" }));

    expect(await screen.findByText("请配置服务商")).toBeInTheDocument();
    expect(screen.getAllByText("请配置服务商")).toHaveLength(1);
    expect(await screen.findByRole("heading", { name: "AI 服务商与模型" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "通用" })).toHaveAttribute("aria-current", "page");
    await waitFor(() => expect(scrollIntoView).toHaveBeenCalled());
  });

  it("acknowledges guidance only after the settings draft is ready and focus is scheduled", async () => {
    const settings = deferred<PublicSettings>();
    const takeSettingsGuidance = vi.fn().mockResolvedValue({
      focus: "providers",
      notice: "请先配置服务商",
    });
    const acknowledge = vi.spyOn(settingsGuidanceInbox, "acknowledge");
    const scheduleFrame = vi.spyOn(window, "requestAnimationFrame");
    stubScrollIntoView();
    window._popper_ = {
      getSettings: vi.fn(() => settings.promise),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "windows",
        trusted: true,
        canRequest: false,
      })),
      requestAccessibility: vi.fn(),
      takeSettingsGuidance,
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await waitFor(() => expect(takeSettingsGuidance).toHaveBeenCalledOnce());
    await act(async () => Promise.resolve());
    expect(acknowledge).not.toHaveBeenCalled();
    expect(screen.queryByText("请先配置服务商")).not.toBeInTheDocument();

    act(() => settings.resolve(DEFAULT_PUBLIC_SETTINGS));

    expect(await screen.findByText("请先配置服务商")).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "AI 服务商与模型" })).toBeInTheDocument();
    expect(scheduleFrame).toHaveBeenCalled();
    expect(acknowledge).toHaveBeenCalledOnce();
  });

  it("defaults to the general section and switches sections from the sidebar nav", async () => {
    installDefaultBridge();
    render(<SettingsApp />);

    expect(await screen.findByText("使用提示")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "通用" })).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: "启用划词助手" })).not.toBeInTheDocument();
    expect(screen.queryByText("工具条显示")).not.toBeInTheDocument();
    expect(screen.queryByText("关闭主窗口时")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "划词获取" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("默认划词获取方式")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "通用" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("heading", { name: "AI 服务商与模型" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "服务商" })).not.toBeInTheDocument();
    expect(screen.queryByText("查看右键长按使用提示与系统选区访问状态。")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "工具栏动作" })).not.toBeInTheDocument();
    expect(screen.queryByRole("radio", { name: "使用百度" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /添加搜索引擎/ })).not.toBeInTheDocument();

    await goToSettingsSection("动作");
    expect(await screen.findByRole("heading", { name: "工具栏动作" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "AI 服务商与模型" })).not.toBeInTheDocument();
  });

  it("places collapsed providers between usage tips and selection access", async () => {
    installDefaultBridge();
    render(<SettingsApp />);

    const providers = await screen.findByRole("heading", { name: "AI 服务商与模型" });
    const tips = screen.getByText("使用提示");
    const selection = screen.getByRole("heading", { name: "Windows 选区访问" });
    expect(tips.compareDocumentPosition(providers) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(providers.compareDocumentPosition(selection) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    const toggle = screen.getByRole("button", { name: "展开OpenAI Compatible配置" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    const baseUrl = screen.getByDisplayValue("https://api.openai.com/v1");
    expect(baseUrl.closest(".provider-grid")).toHaveAttribute("hidden");
    fireEvent.click(toggle);
    expect(baseUrl.closest(".provider-grid")).not.toHaveAttribute("hidden");
    fireEvent.click(screen.getByRole("button", { name: "收起OpenAI Compatible配置" }));
    expect(baseUrl.closest(".provider-grid")).toHaveAttribute("hidden");
  });

  it("expands a new provider and collapses all providers on reopening without losing drafts", async () => {
    let reopen: (() => void) | undefined;
    const unsubscribe = vi.fn();
    installDefaultBridge({
      onSettingsOpened: vi.fn((listener) => {
        reopen = listener;
        return unsubscribe;
      }),
    });
    const view = render(<SettingsApp />);
    await openProviderConfigurations();
    fireEvent.click(screen.getByRole("button", { name: "添加服务商" }));
    expect(screen.getByRole("button", { name: "收起服务商 2配置" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    const inputs = screen.getAllByRole("textbox", { name: /^API 地址（HTTP \/ HTTPS）/ });
    fireEvent.change(inputs[1]!, { target: { value: "https://new.example.com/v1" } });
    const key = document.querySelectorAll<HTMLInputElement>('input[type="password"]')[1]!;
    fireEvent.change(key, { target: { value: "sk-unsaved" } });

    await goToSettingsSection("动作");
    await goToSettingsSection("通用");
    expect(screen.getByRole("button", { name: "收起服务商 2配置" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    act(() => reopen?.());
    expect(screen.getAllByRole("button", { name: /^展开.*配置$/ })).toHaveLength(2);
    expect(screen.queryByRole("button", { name: /^收起.*配置$/ })).not.toBeInTheDocument();
    expect(screen.getByDisplayValue("https://new.example.com/v1")).toBeInTheDocument();
    expect(document.querySelectorAll<HTMLInputElement>('input[type="password"]')[1]).toHaveValue(
      "sk-unsaved",
    );
    fireEvent.click(screen.getByRole("button", { name: "展开服务商 2配置" }));
    expect(
      screen.getByDisplayValue("https://new.example.com/v1").closest(".provider-grid"),
    ).not.toHaveAttribute("hidden");
    view.unmount();
    expect(unsubscribe).toHaveBeenCalledOnce();
  });

  it("collapses saved providers when the settings renderer is recreated", async () => {
    const settings: PublicSettings = {
      ...DEFAULT_PUBLIC_SETTINGS,
      providers: [
        ...DEFAULT_PUBLIC_SETTINGS.providers,
        {
          id: "new-provider",
          name: "服务商 2",
          baseUrl: "https://new.example.com/v1",
          keyConfigured: false,
          enabled: true,
          models: [],
        },
      ],
    };
    installDefaultBridge({ getSettings: vi.fn(async () => settings) });
    const view = render(<SettingsApp />);
    await openProviderConfigurations();
    view.unmount();
    render(<SettingsApp />);
    await screen.findByRole("heading", { name: "AI 服务商与模型" });
    expect(screen.getAllByRole("button", { name: /^展开.*配置$/ })).toHaveLength(2);
    expect(screen.queryByRole("button", { name: /^收起.*配置$/ })).not.toBeInTheDocument();
  });

  it("shows AI default reply language under the language section", async () => {
    installDefaultBridge();
    render(<SettingsApp />);

    await goToSettingsSection("语言");
    expect(await screen.findByRole("combobox", { name: "AI 默认回复语言" })).toBeInTheDocument();
    expect(screen.getByText(/检测到 简体中文 时译为 English/)).toBeInTheDocument();
    expect(screen.queryByText("默认回答语言")).not.toBeInTheDocument();
  });

  it("switches to the actions section when guidance focuses actions", async () => {
    const scrollIntoView = stubScrollIntoView();
    installDefaultBridge({
      takeSettingsGuidance: vi.fn(async () => ({
        focus: "actions" as const,
        notice: "请配置动作",
      })),
    });
    render(<SettingsApp />);

    expect(await screen.findByText("请配置动作")).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "工具栏动作" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "动作" })).toHaveAttribute("aria-current", "page");
    await waitFor(() => expect(scrollIntoView).toHaveBeenCalled());
  });

  it("fades a successful save after 1.4 seconds and removes it after 2 seconds", async () => {
    const updateSettings = vi.fn(async (): Promise<PublicSettings> => DEFAULT_PUBLIC_SETTINGS);
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "windows",
        trusted: true,
        canRequest: false,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    fireEvent.click(await screen.findByRole("button", { name: "保存设置" }));

    await waitFor(() => expect(updateSettings).toHaveBeenCalledOnce());
    const banner = await screen.findByText("设置已保存");
    expect(banner.parentElement).not.toHaveClass("settings-banner--fading");

    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 1_450));
    });
    expect(banner.parentElement).toHaveClass("settings-banner--fading");

    await waitFor(
      () => {
        expect(screen.queryByText("设置已保存")).not.toBeInTheDocument();
      },
      { timeout: 1_000 },
    );
  });

  it("fades an error banner after 1.4 seconds and removes it after 2 seconds", async () => {
    const updateSettings = vi.fn(async (): Promise<PublicSettings> => {
      throw new Error("保存失败了");
    });
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "windows",
        trusted: true,
        canRequest: false,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await screen.findByText("使用提示");
    fireEvent.click(screen.getByRole("button", { name: "保存设置" }));

    const banner = await screen.findByText("保存失败了");
    expect(banner.parentElement).toHaveClass("notice--error");
    expect(banner.parentElement).not.toHaveClass("settings-banner--fading");

    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 1_450));
    });
    expect(banner.parentElement).toHaveClass("settings-banner--fading");

    await waitFor(
      () => {
        expect(screen.queryByText("保存失败了")).not.toBeInTheDocument();
      },
      { timeout: 1_000 },
    );
  });

  it("keeps Windows lifecycle controls out of the macOS settings UI", async () => {
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);

    await screen.findByRole("heading", { name: "辅助功能权限" });
    expect(screen.queryByRole("combobox", { name: "关闭主窗口时" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "退出 Popper" })).not.toBeInTheDocument();
  });

  it("keeps settings editable when the accessibility diagnostic IPC fails", async () => {
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => {
        throw new Error("diagnostic unavailable");
      }),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);

    expect(await screen.findByRole("button", { name: "保存设置" })).toBeEnabled();
    expect(screen.getByText("diagnostic unavailable")).toBeInTheDocument();
  });

  it("shows Windows selection access without macOS permission instructions", async () => {
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "windows",
        trusted: true,
        canRequest: false,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);

    expect(await screen.findByRole("heading", { name: "Windows 选区访问" })).toBeInTheDocument();
    expect(screen.getByText(/不需要单独授权/)).toBeInTheDocument();
    expect(screen.queryByText(/隐私与安全性/)).not.toBeInTheDocument();
  });

  it("shows replayed Windows listener failures", async () => {
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "windows",
        trusted: true,
        canRequest: false,
        available: false,
        diagnostics: {
          selectionMonitorError: "无法启动系统划词监听。请重新启动 Popper。",
        },
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);

    expect(await screen.findByText("选区访问不可用")).toBeInTheDocument();
    expect(screen.getByText("无法启动系统划词监听。请重新启动 Popper。")).toBeInTheDocument();
    expect(screen.queryByText("选区访问可用")).not.toBeInTheDocument();
  });

  it("shows long-press guidance without trigger or capture-shortcut controls", async () => {
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "windows", trusted: true, canRequest: false, available: true, diagnostics: {},
      })),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;
    render(<SettingsApp />);
    expect(await screen.findByRole("note")).toHaveTextContent(/长按右键 250 ms/);
    expect(screen.queryByRole("radiogroup", { name: "触发方式" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("捕获当前选区快捷键")).not.toBeInTheDocument();
  });

  it("refreshes listener diagnostics immediately after save fails", async () => {
    const updateSettings = vi.fn(async () => { throw new Error("保存设置失败"); });
    const getAccessibilityStatus = vi.fn(async () => ({
      platform: "windows", trusted: true, canRequest: false,
      available: updateSettings.mock.calls.length === 0,
      diagnostics: updateSettings.mock.calls.length > 0
        ? { selectionMonitorError: "无法启动系统划词监听。请重新启动 Popper。" } : {},
    }));
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS), updateSettings, getAccessibilityStatus,
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;
    render(<SettingsApp />);
    fireEvent.click(await screen.findByRole("button", { name: "保存设置" }));
    expect(await screen.findByText("无法启动系统划词监听。请重新启动 Popper。")).toBeInTheDocument();
    expect(getAccessibilityStatus).toHaveBeenCalledTimes(2);
  });

  it("reorders toolbar actions with the keyboard drag handle and persists order values", async () => {
    const rowOrder = new Map(
      DEFAULT_PUBLIC_SETTINGS.actions.map((action, index) => [action.id, index]),
    );
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      const element = this;
      const row = element.closest<HTMLElement>("[data-action-id]");
      const actionId = row?.dataset.actionId;
      const zone = element.closest<HTMLElement>("[data-action-zone]");
      const index = actionId ? (rowOrder.get(actionId) ?? 0) : 0;
      const left = zone?.dataset.actionZone === "disabled" ? 420 : 20;
      const top = actionId ? 500 + index * 64 : 480;
      const width = actionId ? 360 : 380;
      const height = actionId ? 54 : 460;
      return {
        x: left,
        y: top,
        left,
        top,
        right: left + width,
        bottom: top + height,
        width,
        height,
        toJSON: () => ({}),
      } as DOMRect;
    });

    const updateSettings = vi.fn(
      async (update: SettingsUpdate): Promise<PublicSettings> => ({
        ...DEFAULT_PUBLIC_SETTINGS,
        ...update,
        providers: (update.providers ?? DEFAULT_PUBLIC_SETTINGS.providers).map((provider) => ({
          ...provider,
          keyConfigured: false,
        })),
        actions: update.actions ?? DEFAULT_PUBLIC_SETTINGS.actions,
      }),
    );
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await goToSettingsSection("动作");

    const handle = await screen.findByRole("button", { name: "拖动复制" });
    handle.focus();
    fireEvent.keyDown(handle, { key: " ", code: "Space" });
    await waitFor(() => {
      expect(document.body.textContent).toContain(
        "Draggable item copy was moved over droppable area copy",
      );
    });
    fireEvent.keyDown(document, { key: "ArrowUp", code: "ArrowUp" });
    await waitFor(() => {
      expect(document.body.textContent).toContain(
        "Draggable item copy was moved over droppable area search",
      );
    });
    fireEvent.keyDown(document, { key: " ", code: "Space" });

    await waitFor(() => {
      const enabledZone = document.querySelector<HTMLElement>('[data-action-zone="enabled"]');
      expect(
        [...(enabledZone?.querySelectorAll<HTMLElement>("[data-action-id]") ?? [])].map(
          (row) => row.dataset.actionId,
        ),
      ).toEqual(["translate", "explain", "summary", "copy", "search"]);
    });

    fireEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(
      (updateSettings.mock.calls[0]?.[0].actions ?? []).map(({ id, order }) => ({ id, order })),
    ).toEqual([
      { id: "translate", order: 0 },
      { id: "explain", order: 1 },
      { id: "summary", order: 2 },
      { id: "copy", order: 3 },
      { id: "search", order: 4 },
      { id: "refine", order: 5 },
    ]);
  });

  it("shows a non-blocking plaintext warning for HTTP provider addresses", async () => {
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();

    const baseUrl = await screen.findByDisplayValue("https://api.openai.com/v1");
    expect(screen.queryByText(/API Key 将通过网络明文传输/)).not.toBeInTheDocument();
    fireEvent.change(baseUrl, { target: { value: "http://api.example.com/v1" } });
    expect(screen.getByText(/API Key 将通过网络明文传输/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保存设置" })).toBeEnabled();
  });

  it("edits the persisted result text size through the supported range control", async () => {
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await goToSettingsSection("结果");

    const textSize = await screen.findByRole("slider", { name: "结果文字大小" });
    expect(textSize).toHaveValue("14");
    fireEvent.change(textSize, { target: { value: "18" } });
    expect(textSize).toHaveValue("18");
    expect(screen.getByText("18 px")).toBeInTheDocument();
  });

  it("shows the Cherry default prompt and lets the user edit it", async () => {
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await goToSettingsSection("动作");

    fireEvent.click(await screen.findByRole("button", { name: "编辑翻译" }));
    const prompt = document.querySelector<HTMLTextAreaElement>("#action-prompt");
    expect(prompt).toHaveValue(DEFAULT_ACTION_PROMPTS.translate);

    fireEvent.change(prompt!, { target: { value: "我的翻译规则：{{text}}" } });
    fireEvent.click(screen.getByRole("button", { name: "保存修改" }));

    expect(screen.getByText("提示词：我的翻译规则：{{text}}")).toBeInTheDocument();
  });

  it("uses an in-app confirmation and removes providers in the ordinary settings commit", async () => {
    const deletedSettings = withoutDefaultProvider();
    const deleteProvider = vi.fn(async () => deletedSettings);
    const updateSettings = vi.fn(async (_update: SettingsUpdate) => deletedSettings);
    const browserConfirm = vi.spyOn(window, "confirm");

    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings,
      deleteProvider,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();

    const removeButton = await screen.findByRole("button", { name: /删除OpenAI Compatible/ });
    fireEvent.click(removeButton);

    expect(screen.getByRole("dialog", { name: /删除“OpenAI Compatible”/ })).toBeInTheDocument();
    expect(browserConfirm).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "删除服务商" }));
    expect(screen.queryByRole("button", { name: /删除OpenAI Compatible/ })).not.toBeInTheDocument();
    expect(screen.getByText(/保存设置后生效/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(updateSettings).toHaveBeenCalledTimes(1);
    });
    expect(deleteProvider).not.toHaveBeenCalled();
    expect(updateSettings.mock.calls[0]?.[0].providers).toEqual([]);
    expect(
      updateSettings.mock.calls[0]?.[0].actions
        ?.filter((action) => "providerId" in action)
        .every((action) => action.providerId === "" && action.modelId === ""),
    ).toBe(true);
  });

  it("preserves unsaved providers, keys and fields when another provider key is cleared", async () => {
    const configured = {
      ...DEFAULT_PUBLIC_SETTINGS,
      providers: DEFAULT_PUBLIC_SETTINGS.providers.map((provider) => ({
        ...provider,
        keyConfigured: true,
      })),
    };
    const backendCleared = {
      ...configured,
      providers: configured.providers.map((provider) => ({
        ...provider,
        keyConfigured: false,
      })),
    };
    const clearProviderApiKey = vi.fn(async () => backendCleared);

    window._popper_ = {
      getSettings: vi.fn(async () => configured),
      updateSettings: vi.fn(),
      clearProviderApiKey,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();

    const baseUrl = await screen.findByDisplayValue("https://api.openai.com/v1");
    fireEvent.change(baseUrl, { target: { value: "https://gateway.example/v1" } });
    fireEvent.click(screen.getByRole("button", { name: "添加服务商" }));
    expect(screen.getByDisplayValue("服务商 2")).toBeInTheDocument();

    const keyInputs = document.querySelectorAll<HTMLInputElement>('input[type="password"]');
    expect(keyInputs).toHaveLength(2);
    fireEvent.change(keyInputs[1]!, { target: { value: "sk-new-provider" } });
    const clearKey = screen
      .getAllByRole("button", { name: "清除密钥" })
      .find((button) => !(button as HTMLButtonElement).disabled);
    expect(clearKey).toBeDefined();
    fireEvent.click(clearKey!);

    await waitFor(() => expect(clearProviderApiKey).toHaveBeenCalledWith(DEFAULT_PROVIDER_ID));
    expect(screen.getByDisplayValue("https://gateway.example/v1")).toBeInTheDocument();
    expect(screen.getByDisplayValue("服务商 2")).toBeInTheDocument();
    expect(document.querySelectorAll<HTMLInputElement>('input[type="password"]')[1]).toHaveValue(
      "sk-new-provider",
    );
  });

  it("uses an in-app confirmation when removing an action", async () => {
    const browserConfirm = vi.spyOn(window, "confirm");
    window._popper_ = {
      getSettings: vi.fn(async () => DEFAULT_PUBLIC_SETTINGS),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await goToSettingsSection("动作");

    fireEvent.click(await screen.findByRole("button", { name: "移除润色" }));
    expect(screen.getByRole("dialog", { name: "移除动作“润色”？" })).toBeInTheDocument();
    expect(browserConfirm).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "移除动作" }));
    expect(screen.queryByRole("button", { name: "移除润色" })).not.toBeInTheDocument();
    expect(screen.getByText(/已移除动作“润色”/)).toBeInTheDocument();
  });

  it("commits ordinary settings first and keeps recoverable key input after a key failure", async () => {
    let persisted: PublicSettings | null = null;
    const getSettings = vi.fn(async () => persisted ?? DEFAULT_PUBLIC_SETTINGS);
    const updateSettings = vi.fn(async (update: SettingsUpdate) => {
      persisted = {
        ...DEFAULT_PUBLIC_SETTINGS,
        ...update,
        providers: (update.providers ?? []).map((provider) => ({
          ...provider,
          keyConfigured: false,
        })),
        actions: update.actions ?? DEFAULT_PUBLIC_SETTINGS.actions,
      };
      return persisted;
    });
    const setProviderApiKey = vi.fn(async () => {
      throw new Error("Encrypted secret store unavailable");
    });

    window._popper_ = {
      getSettings,
      updateSettings,
      setProviderApiKey,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();

    await screen.findByDisplayValue("OpenAI Compatible");
    fireEvent.click(screen.getByRole("button", { name: "添加服务商" }));
    const keyInputs = document.querySelectorAll<HTMLInputElement>('input[type="password"]');
    fireEvent.change(keyInputs[1]!, { target: { value: "sk-retry-me" } });
    fireEvent.click(screen.getByRole("button", { name: "保存设置" }));

    expect(await screen.findByText(/普通设置已保存，但 API Key 保存失败/)).toBeInTheDocument();
    expect(updateSettings).toHaveBeenCalledTimes(1);
    expect(setProviderApiKey).toHaveBeenCalledTimes(1);
    expect(updateSettings.mock.invocationCallOrder[0]).toBeLessThan(
      setProviderApiKey.mock.invocationCallOrder[0]!,
    );
    expect(screen.getByDisplayValue("服务商 2")).toBeInTheDocument();
    expect(document.querySelectorAll<HTMLInputElement>('input[type="password"]')[1]).toHaveValue(
      "sk-retry-me",
    );
  });

  it("does not fetch saved API keys until the user clicks show", async () => {
    const getProviderApiKey = vi.fn(async () => "sk-secret-should-not-preload");
    const configured: PublicSettings = {
      ...DEFAULT_PUBLIC_SETTINGS,
      providers: DEFAULT_PUBLIC_SETTINGS.providers.map((provider) => ({
        ...provider,
        keyConfigured: true,
      })),
    };
    window._popper_ = {
      getSettings: vi.fn(async () => configured),
      updateSettings: vi.fn(),
      getProviderApiKey,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();

    await waitFor(() => {
      expect(screen.getByText(/AI 服务商/)).toBeInTheDocument();
    });
    expect(getProviderApiKey).not.toHaveBeenCalled();
  });

  it("loads the saved API key when the user clicks show", async () => {
    const getProviderApiKey = vi.fn(async () => "sk-revealed-key");
    const configured: PublicSettings = {
      ...DEFAULT_PUBLIC_SETTINGS,
      providers: DEFAULT_PUBLIC_SETTINGS.providers.map((provider) => ({
        ...provider,
        keyConfigured: true,
      })),
    };
    window._popper_ = {
      getSettings: vi.fn(async () => configured),
      updateSettings: vi.fn(),
      getProviderApiKey,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();

    const toggle = await screen.findByRole("button", { name: /显示 API Key/ });
    fireEvent.click(toggle);
    await waitFor(() => {
      expect(getProviderApiKey).toHaveBeenCalled();
    });
    const input = screen.getByDisplayValue("sk-revealed-key");
    expect(input).toHaveAttribute("type", "text");
  });

  it("shows provider models as vertical reorderable rows without thinking badges", async () => {
    const settingsWithModels: PublicSettings = {
      ...DEFAULT_PUBLIC_SETTINGS,
      providers: DEFAULT_PUBLIC_SETTINGS.providers.map((provider) => ({
        ...provider,
        models: [
          { id: "gpt-4o-mini", name: "gpt-4o-mini", thinkingLevels: [] },
          { id: "o3-mini", name: "o3-mini", thinkingLevels: ["low", "medium", "high"] },
        ],
      })),
    };
    window._popper_ = {
      getSettings: vi.fn(async () => settingsWithModels),
      updateSettings: vi.fn(),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();

    expect(await screen.findByRole("button", { name: "获取模型" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "同步模型" })).not.toBeInTheDocument();

    expect(await screen.findByRole("button", { name: "拖动模型 gpt-4o-mini" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "拖动模型 o3-mini" })).toBeInTheDocument();
    expect(screen.getByText("gpt-4o-mini", { selector: ".model-row__name" })).toBeInTheDocument();
    expect(screen.getByText("o3-mini", { selector: ".model-row__name" })).toBeInTheDocument();
    expect(document.querySelector(".model-row-list")).toBeInTheDocument();

    // Provider model list must not show thinking badges
    expect(screen.queryByText(/思考\s*:/)).not.toBeInTheDocument();
    expect(screen.queryByText(/思考:\s*低/)).not.toBeInTheDocument();

    expect(screen.getAllByRole("button", { name: /拖动模型/ }).length).toBeGreaterThan(0);
    expect(screen.getAllByRole("button", { name: /移除模型/ }).length).toBeGreaterThan(0);
  });

  it("fetches models into a multi-select picker and merges the subset into the draft", async () => {
    const settingsWithModels: PublicSettings = {
      ...DEFAULT_PUBLIC_SETTINGS,
      providers: DEFAULT_PUBLIC_SETTINGS.providers.map((provider) => ({
        ...provider,
        models: [
          { id: "gpt-4o-mini", name: "gpt-4o-mini", thinkingLevels: [] },
          { id: "manual-keep", name: "manual-keep", thinkingLevels: [] },
        ],
      })),
    };
    const listProviderModels = vi.fn(async () => ({
      ok: true as const,
      models: [
        { id: "gpt-4o-mini", name: "GPT-4o Mini", thinkingLevels: [] },
        { id: "gpt-4o", name: "GPT-4o", thinkingLevels: [] },
        { id: "o3-mini", name: "o3-mini", thinkingLevels: ["low", "medium", "high"] },
      ],
    }));
    const syncProviderModels = vi.fn();
    const updateSettings = vi.fn(
      async (update: SettingsUpdate): Promise<PublicSettings> => ({
        ...settingsWithModels,
        ...update,
        providers: (update.providers ?? settingsWithModels.providers).map((provider) => ({
          ...provider,
          keyConfigured: false,
        })),
        actions: update.actions ?? settingsWithModels.actions,
      }),
    );
    window._popper_ = {
      getSettings: vi.fn(async () => settingsWithModels),
      updateSettings,
      listProviderModels,
      syncProviderModels,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();

    fireEvent.click(await screen.findByRole("button", { name: "获取模型" }));
    expect(await screen.findByRole("dialog", { name: "选择模型" })).toBeInTheDocument();
    await waitFor(() => expect(listProviderModels).toHaveBeenCalled());
    expect(syncProviderModels).not.toHaveBeenCalled();

    const gpt4oMini = screen.getByRole("checkbox", { name: /GPT-4o Mini/i });
    const gpt4o = screen.getByRole("checkbox", { name: /GPT-4o$/i });
    const o3 = screen.getByRole("checkbox", { name: /o3-mini/i });
    const manual = screen.getByRole("checkbox", { name: /manual-keep/i });
    expect(gpt4oMini).toBeChecked();
    expect(manual).toBeChecked();
    expect(gpt4o).not.toBeChecked();
    expect(o3).not.toBeChecked();

    fireEvent.click(gpt4o);
    fireEvent.click(gpt4oMini);
    fireEvent.click(screen.getByRole("button", { name: /应用所选/ }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "选择模型" })).not.toBeInTheDocument();
    });
    expect(screen.getByText(/已选择 2 个模型，请保存设置/)).toBeInTheDocument();
    expect(screen.getByText("GPT-4o", { selector: ".model-row__name" })).toBeInTheDocument();
    expect(screen.getByText("manual-keep", { selector: ".model-row__name" })).toBeInTheDocument();
    expect(
      screen.queryByText("gpt-4o-mini", { selector: ".model-row__name" }),
    ).not.toBeInTheDocument();
    expect(syncProviderModels).not.toHaveBeenCalled();
  });

  it("cancels the model picker without changing draft models", async () => {
    const settingsWithModels: PublicSettings = {
      ...DEFAULT_PUBLIC_SETTINGS,
      providers: DEFAULT_PUBLIC_SETTINGS.providers.map((provider) => ({
        ...provider,
        models: [{ id: "gpt-4o-mini", name: "gpt-4o-mini", thinkingLevels: [] }],
      })),
    };
    window._popper_ = {
      getSettings: vi.fn(async () => settingsWithModels),
      updateSettings: vi.fn(async () => settingsWithModels),
      listProviderModels: vi.fn(async () => ({
        ok: true as const,
        models: [
          { id: "gpt-4o-mini", name: "gpt-4o-mini", thinkingLevels: [] },
          { id: "gpt-4o", name: "gpt-4o", thinkingLevels: [] },
        ],
      })),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();
    fireEvent.click(await screen.findByRole("button", { name: "获取模型" }));
    expect(await screen.findByRole("dialog", { name: "选择模型" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox", { name: /gpt-4o$/i }));
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "选择模型" })).not.toBeInTheDocument();
    });
    expect(screen.getByText("gpt-4o-mini", { selector: ".model-row__name" })).toBeInTheDocument();
    expect(screen.queryByText("gpt-4o", { selector: ".model-row__name" })).not.toBeInTheDocument();
  });

  it("shows an error banner when model fetch fails and leaves models unchanged", async () => {
    const settingsWithModels: PublicSettings = {
      ...DEFAULT_PUBLIC_SETTINGS,
      providers: DEFAULT_PUBLIC_SETTINGS.providers.map((provider) => ({
        ...provider,
        models: [{ id: "gpt-4o-mini", name: "gpt-4o-mini", thinkingLevels: [] }],
      })),
    };
    window._popper_ = {
      getSettings: vi.fn(async () => settingsWithModels),
      updateSettings: vi.fn(async () => settingsWithModels),
      listProviderModels: vi.fn(async () => ({
        ok: false as const,
        message: "无法连接服务商",
      })),
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();
    fireEvent.click(await screen.findByRole("button", { name: "获取模型" }));
    expect(await screen.findByText("无法连接服务商")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "选择模型" })).not.toBeInTheDocument();
    expect(screen.getByText("gpt-4o-mini", { selector: ".model-row__name" })).toBeInTheDocument();
  });

  it("reorders provider models with the keyboard drag handle", async () => {
    const modelIds = ["gpt-4o-mini", "o3-mini"];
    const modelOrder = new Map<string, number>(modelIds.map((id, index) => [id, index]));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      const row = this.closest<HTMLElement>("[data-model-id]");
      const modelId = row?.dataset.modelId;
      const index = modelId ? (modelOrder.get(modelId) ?? 0) : 0;
      const left = 20;
      const top = modelId ? 500 + index * 64 : 480;
      const width = modelId ? 360 : 380;
      const height = modelId ? 54 : 200;
      return {
        x: left,
        y: top,
        left,
        top,
        right: left + width,
        bottom: top + height,
        width,
        height,
        toJSON: () => ({}),
      } as DOMRect;
    });

    const settingsWithModels: PublicSettings = {
      ...DEFAULT_PUBLIC_SETTINGS,
      providers: DEFAULT_PUBLIC_SETTINGS.providers.map((provider) => ({
        ...provider,
        models: [
          { id: "gpt-4o-mini", name: "gpt-4o-mini", thinkingLevels: [] },
          { id: "o3-mini", name: "o3-mini", thinkingLevels: ["low", "medium", "high"] },
        ],
      })),
    };
    const updateSettings = vi.fn(
      async (update: SettingsUpdate): Promise<PublicSettings> => ({
        ...settingsWithModels,
        ...update,
        providers: (update.providers ?? settingsWithModels.providers).map((provider) => ({
          ...provider,
          keyConfigured: false,
        })),
        actions: update.actions ?? settingsWithModels.actions,
      }),
    );
    window._popper_ = {
      getSettings: vi.fn(async () => settingsWithModels),
      updateSettings,
      getAccessibilityStatus: vi.fn(async () => ({
        platform: "darwin",
        trusted: true,
        canRequest: true,
      })),
      requestAccessibility: vi.fn(),
      onSettingsChanged: vi.fn(() => () => undefined),
    } as unknown as WindowPopperApi;

    render(<SettingsApp />);
    await openProviderConfigurations();

    const handle = await screen.findByRole("button", { name: "拖动模型 gpt-4o-mini" });
    handle.focus();
    fireEvent.keyDown(handle, { key: " ", code: "Space" });
    await waitFor(() => {
      expect(document.body.textContent).toContain(
        "Draggable item gpt-4o-mini was moved over droppable area gpt-4o-mini",
      );
    });
    fireEvent.keyDown(document, { key: "ArrowDown", code: "ArrowDown" });
    await waitFor(() => {
      expect(document.body.textContent).toContain(
        "Draggable item gpt-4o-mini was moved over droppable area o3-mini",
      );
    });
    fireEvent.keyDown(document, { key: " ", code: "Space" });

    await waitFor(() => {
      const dragHandles = screen.getAllByRole("button", { name: /拖动模型/ });
      expect(dragHandles.map((button) => button.getAttribute("aria-label"))).toEqual([
        "拖动模型 o3-mini",
        "拖动模型 gpt-4o-mini",
      ]);
    });

    fireEvent.click(screen.getByRole("button", { name: "保存设置" }));
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(
      (updateSettings.mock.calls[0]?.[0].providers?.[0]?.models ?? []).map((model) => model.id),
    ).toEqual(["o3-mini", "gpt-4o-mini"]);
  });
});
