import { ShortcutRecorder } from "./ShortcutRecorder";
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Crop,
  Eye,
  EyeOff,
  Globe,
  GripVertical,
  KeyRound,
  LoaderCircle,
  LockKeyhole,
  Languages,
  ListFilter,
  Monitor,
  Pencil,
  Plus,
  RefreshCw,
  ShieldBan,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  WandSparkles,
  X,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type JSX,
} from "react";
import { DictionarySettings } from "./DictionarySettings";
import appIcon from "../../../assets/popper.png";

import {
  APP_NAME,
  DEFAULT_APP_SETTINGS,
  DEFAULT_OPENAI_BASE_URL,
  MAX_ENABLED_ACTIONS,
  RESULT_FONT_SIZE_MAX,
  RESULT_FONT_SIZE_MIN,
  TRANSLATION_LANGUAGES,
  TRANSLATION_LANGUAGE_NAMES,
  actionDefinitionSchema,
  isAiActionDefinition,
  requiresProviderApiKey,
  type AccessibilityStatus,
  type ActionDefinition,
  type ProviderModel,
  type PublicProviderSettings,
  type PublicSettings,
  type SettingsGuidance,
  type SupportedLocale,
  type TranslationLanguage,
} from "../../shared";
import { ActionIcon } from "../components/ActionIcon";
import { getToolbarActions, ToolbarActions } from "../components/ToolbarActions";
import { runDetached } from "../lib/asyncEffects";
import { getErrorMessage } from "../lib/errors";
import { useSelectionDetectionDebug } from "../lib/useSelectionDetectionDebug";
import { CustomActionDialog, type ActionEditorValue } from "./CustomActionDialog";
import { mergeProviderModelsOnPick } from "./providerModels";
import {
  buildSettingsUpdate,
  moveActionToZone,
  removeProviderFromSettings,
  removeProviderModelFromSettings,
  sortAndNumberActions,
} from "./settingsForm";
import { settingsGuidanceInbox, type SettingsGuidanceLease } from "./settingsGuidanceInbox";
import { StyledSelect } from "../components/StyledSelect";

type Operation = string | null;
type Banner = { kind: "success" | "error"; text: string } | null;
type ModelPickerState = {
  providerId: string;
  providerName: string;
  previous: ProviderModel[];
  remoteModels: ProviderModel[];
  checkedIds: Set<string>;
};
type SettingsSectionId = "general" | "actions" | "screen" | "language";
type SelectionSubTab = "actions" | "result" | "filter";

const SETTINGS_SECTIONS: ReadonlyArray<{
  id: SettingsSectionId;
  label: string;
  blurb: string;
  icon: typeof Sparkles;
  caption: string;
}> = [
  {
    id: "general",
    label: "通用",
    blurb: "",
    icon: SlidersHorizontal,
    caption: "服务商与选区访问",
  },
  {
    id: "actions",
    label: "动作",
    blurb: "配置划词动作、快捷提问、结果窗口展示行为与应用过滤规则。",
    icon: WandSparkles,
    caption: "划词动作与快捷提问",
  },
  {
    id: "screen",
    label: "屏幕",
    blurb: "配置屏幕截图、标注工具、浮动贴图与全局快捷键。",
    icon: Crop,
    caption: "截图、标注与贴图",
  },
  {
    id: "language",
    label: "语言",
    blurb: "设置 AI 默认回复语言，以及翻译的源语言与目标语言。",
    icon: Languages,
    caption: "回复与翻译偏好",
  },
];

const SETTINGS_ACTIVE_SECTION_STORAGE_KEY = "popper.settings.active-section";
const SETTINGS_SELECTION_SUBTAB_STORAGE_KEY = "popper.settings.selection-subtab";

function restoredSettingsSection(): SettingsSectionId {
  try {
    const value = window.sessionStorage.getItem(SETTINGS_ACTIVE_SECTION_STORAGE_KEY);
    return SETTINGS_SECTIONS.some((section) => section.id === value)
      ? (value as SettingsSectionId)
      : "general";
  } catch {
    return "general";
  }
}

function restoredSelectionSubTab(): SelectionSubTab {
  try {
    const value = window.sessionStorage.getItem(SETTINGS_SELECTION_SUBTAB_STORAGE_KEY);
    return value === "actions" || value === "result" || value === "filter" ? value : "actions";
  } catch {
    return "actions";
  }
}

const ACTION_KIND_NAMES: Readonly<Record<ActionDefinition["kind"], string>> = {
  translate: "翻译",
  summary: "总结",
  explain: "解释",
  refine: "润色",
  ask: "问AI",
  custom: "自定义 AI",
};

const ACTION_ZONE_IDS = {
  enabled: "action-zone-enabled",
  disabled: "action-zone-disabled",
} as const;

function actionZoneId(enabled: boolean): string {
  return enabled ? ACTION_ZONE_IDS.enabled : ACTION_ZONE_IDS.disabled;
}

function createId(prefix: string): string {
  if (typeof crypto.randomUUID === "function") return `${prefix}-${crypto.randomUUID()}`;
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function normalizePublicSettings(settings: PublicSettings): PublicSettings {
  return { ...settings, actions: sortAndNumberActions(settings.actions) };
}

function normalizedModels(models: readonly ProviderModel[]): ProviderModel[] {
  const seen = new Set<string>();
  return models.flatMap((model) => {
    const id = model.id.trim();
    if (!id || seen.has(id)) return [];
    seen.add(id);
    return [
      {
        id,
        name: model.name.trim() || id,
        thinkingLevels: model.thinkingLevels ?? [],
        thinkingCapability: model.thinkingCapability,
      },
    ];
  });
}

function usesPlainHttp(value: string): boolean {
  try {
    return new URL(value.trim()).protocol === "http:";
  } catch {
    return /^http:/iu.test(value.trim());
  }
}

function accessibilityPresentation(status: AccessibilityStatus | null): {
  heading: string;
  description: string;
  state: string;
  detail: string;
  button: string;
} {
  const available = status?.available ?? false;
  const monitorError = status?.diagnostics?.selectionMonitorError;
  if (status?.platform === "windows") {
    return {
      heading: "Windows 选区访问",
      description:
        "优先通过 Windows UI Automation、Legacy UIA 与 MSAA 读取选区；不会改动系统剪贴板，也不需要单独授权。",
      state: available ? "选区访问可用" : "选区访问不可用",
      detail:
        monitorError ??
        (available
          ? `${APP_NAME} 可以响应新的文本选择。`
          : "当前系统无法读取其他应用的选区，请重新启动应用或检查系统策略。"),
      button: available ? "可用" : "不可用",
    };
  }
  if (status?.platform === "unsupported") {
    return {
      heading: "选区访问",
      description: "当前操作系统尚未提供划词捕获支持。",
      state: "当前平台不受支持",
      detail: "划词动作需要先支持此平台的原生选区接口。",
      button: "不受支持",
    };
  }
  return {
    heading: "辅助功能权限",
    description: "macOS 需要此权限才能识别其他应用里的选中文字。",
    state: status?.trusted ? (available ? "已获得权限" : "划词监听不可用") : "尚未获得权限",
    detail:
      monitorError ??
      (status?.trusted
        ? `${APP_NAME} 可以响应新的文本选择。`
        : `请在“隐私与安全性 → 辅助功能”中允许 ${APP_NAME}。`),
    button: status?.trusted ? (available ? "已启用" : "不可用") : "打开系统设置",
  };
}

export function SettingsApp(): JSX.Element {
  const selectionDebug = useSelectionDetectionDebug();
  const [debugUpdating, setDebugUpdating] = useState(false);
  const [draft, setDraft] = useState<PublicSettings | null>(null);
  const [keyInputs, setKeyInputs] = useState<Record<string, string>>({});
  const [keyBaselines, setKeyBaselines] = useState<Record<string, string>>({});
  const [keyVisible, setKeyVisible] = useState<Record<string, boolean>>({});
  const [newModelInputs, setNewModelInputs] = useState<Record<string, string>>({});
  const [expandedProviders, setExpandedProviders] = useState<Record<string, boolean>>({});
  const [draggingModelId, setDraggingModelId] = useState<string | null>(null);
  const [accessibility, setAccessibility] = useState<AccessibilityStatus | null>(null);
  const [operation, setOperation] = useState<Operation>(null);
  const [banner, setBanner] = useState<Banner>(null);
  const [bannerFading, setBannerFading] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [editor, setEditor] = useState<ActionDefinition | "new" | null>(null);
  const [providerPendingDelete, setProviderPendingDelete] = useState<PublicProviderSettings | null>(
    null,
  );
  const [actionPendingDelete, setActionPendingDelete] = useState<ActionDefinition | null>(null);
  const [modelPicker, setModelPicker] = useState<ModelPickerState | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [activeSection, setActiveSection] = useState<SettingsSectionId>(restoredSettingsSection);
  const [selectionSubTab, setSelectionSubTab] = useState<SelectionSubTab>(restoredSelectionSubTab);
  const handleSelectSubTab = useCallback((subTab: SelectionSubTab) => {
    setSelectionSubTab(subTab);
    try {
      window.sessionStorage.setItem(SETTINGS_SELECTION_SUBTAB_STORAGE_KEY, subTab);
    } catch {
      // ignore
    }
  }, []);
  const activeSectionRef = useRef(activeSection);
  activeSectionRef.current = activeSection;
  const dirtyRef = useRef(false);
  const unsavedChangesRef = useRef(false);
  // Tracks in-flight lazy key reveals so re-entry is ignored and the eye can disable.
  const keyRevealInFlightRef = useRef(new Set<string>());
  const [keyRevealInFlight, setKeyRevealInFlight] = useState<Record<string, boolean>>({});
  const keyInputsRef = useRef(keyInputs);
  keyInputsRef.current = keyInputs;
  const dragSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  useEffect(() => {
    dirtyRef.current = dirty;
  }, [dirty]);

  useEffect(() => {
    // Bringing an already open panel forward collapses providers while
    // preserving its current draft. Closing the panel releases this renderer.
    return window._popper_.onSettingsOpened?.(() => setExpandedProviders({}));
  }, []);

  const pendingGuidanceLeaseRef = useRef<SettingsGuidanceLease | null>(null);
  const draftReadyRef = useRef(false);

  useEffect(() => {
    draftReadyRef.current = draft !== null;
  }, [draft]);

  useEffect(() => {
    // A hidden settings WebView normally stays mounted, preserving unsaved
    // form values (including API-key input). Keep only the active section as
    // a fallback for a later renderer recreation; never persist form data.
    const persistActiveSection = (): void => {
      if (document.visibilityState !== "hidden") return;
      try {
        window.sessionStorage.setItem(
          SETTINGS_ACTIVE_SECTION_STORAGE_KEY,
          activeSectionRef.current,
        );
      } catch {
        // Storage may be unavailable in hardened WebView configurations.
      }
    };
    document.addEventListener("visibilitychange", persistActiveSection);
    window.addEventListener("pagehide", persistActiveSection);
    return () => {
      document.removeEventListener("visibilitychange", persistActiveSection);
      window.removeEventListener("pagehide", persistActiveSection);
    };
  }, []);

  const applySettingsGuidance = useCallback((lease: SettingsGuidanceLease): void => {
    if (!draftReadyRef.current) {
      if (!pendingGuidanceLeaseRef.current) pendingGuidanceLeaseRef.current = lease;
      return;
    }
    if (pendingGuidanceLeaseRef.current && pendingGuidanceLeaseRef.current.id !== lease.id) return;

    pendingGuidanceLeaseRef.current = null;
    const guidance: SettingsGuidance = lease.value;
    const focus = guidance.focus?.trim();
    let targetId: string | null = null;
    if (focus === "providers") {
      setActiveSection("general");
      targetId = "providers-title";
    } else if (focus === "actions") {
      setActiveSection("actions");
      setSelectionSubTab("actions");
      targetId = "actions-title";
    } else if (focus === "result") {
      setActiveSection("actions");
      setSelectionSubTab("result");
      targetId = "result-title";
    } else if (focus === "filter") {
      setActiveSection("actions");
      setSelectionSubTab("filter");
      targetId = "filter-title";
    } else if (focus === "quick-ask") {
      setActiveSection("actions");
      targetId = "quick-ask-title";
    } else if (focus === "screen") {
      setActiveSection("screen");
      targetId = "capture-title";
    }
    if (targetId) {
      const scrollTargetId = targetId;
      const scroll = (): void => {
        document
          .getElementById(scrollTargetId)
          ?.scrollIntoView({ behavior: "smooth", block: "start" });
      };
      // Wait a frame so the focused section has mounted after the switch.
      window.requestAnimationFrame(() => {
        window.requestAnimationFrame(scroll);
      });
    }
    const notice = guidance.notice?.trim();
    if (notice) {
      setBanner({ kind: "success", text: notice });
    }
    settingsGuidanceInbox.acknowledge(lease.id);
    runDetached(
      settingsGuidanceInbox
        .acquire(async () => null)
        .then((nextLease) => {
          if (nextLease && nextLease.id !== lease.id) applySettingsGuidance(nextLease);
        }),
      {
        scope: "settings",
        operation: "apply-guidance",
      },
    );
  }, []);

  useEffect(() => {
    let disposed = false;
    const take = window._popper_.takeSettingsGuidance;
    if (take) {
      runDetached(
        settingsGuidanceInbox.acquire(take).then((lease) => {
          if (!disposed && lease) applySettingsGuidance(lease);
        }),
        {
          scope: "settings",
          operation: "take-guidance",
        },
      );
    }
    const unsubscribe = window._popper_.onSettingsGuidance?.((guidance) => {
      const lease = settingsGuidanceInbox.push(guidance);
      if (!disposed) applySettingsGuidance(lease);
    });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [applySettingsGuidance]);

  useEffect(() => {
    const pendingLease = pendingGuidanceLeaseRef.current;
    if (!draft || !pendingLease) return;
    applySettingsGuidance(pendingLease);
  }, [draft, applySettingsGuidance]);

  useEffect(() => {
    if (!banner) {
      setBannerFading(false);
      return;
    }

    setBannerFading(false);
    // Shared toast lifetime: fade starts at 1.4s, remove at 2.0s (both green and red).
    const fadeTimer = window.setTimeout(() => setBannerFading(true), 1_400);
    const removeTimer = window.setTimeout(() => {
      setBanner(null);
      setBannerFading(false);
    }, 2_000);
    return () => {
      window.clearTimeout(fadeTimer);
      window.clearTimeout(removeTimer);
    };
  }, [banner]);

  const keysDirty = useMemo(() => {
    const ids = new Set([...Object.keys(keyInputs), ...Object.keys(keyBaselines)]);
    for (const id of ids) {
      if ((keyInputs[id] ?? "") !== (keyBaselines[id] ?? "")) return true;
    }
    return false;
  }, [keyInputs, keyBaselines]);
  unsavedChangesRef.current = dirty || keysDirty;

  const refreshRuntimeStatus = useCallback(async (): Promise<void> => {
    const status = await window._popper_.getAccessibilityStatus();
    setAccessibility(status);
  }, []);

  // Clear local key fields without fetching secrets. Saved keys stay in the
  // secret store; the form shows a placeholder until the user clicks show.
  const resetProviderKeyFields = useCallback((): void => {
    setKeyInputs({});
    setKeyBaselines({});
    setKeyVisible({});
  }, []);

  const revealProviderApiKey = useCallback(
    async (provider: PublicProviderSettings): Promise<void> => {
      if (keyVisible[provider.id]) {
        // Hide only — never clear dirty user input.
        setKeyVisible((current) => ({ ...current, [provider.id]: false }));
        return;
      }

      // Ignore re-entry while a reveal for this provider is already in flight.
      if (keyRevealInFlightRef.current.has(provider.id)) return;

      const currentInput = keyInputs[provider.id] ?? "";
      if (currentInput === "" && provider.keyConfigured && window._popper_.getProviderApiKey) {
        keyRevealInFlightRef.current.add(provider.id);
        setKeyRevealInFlight((current) => ({ ...current, [provider.id]: true }));
        try {
          const key = await window._popper_.getProviderApiKey(provider.id);
          if (typeof key === "string" && key.length > 0) {
            // Only fill fields that are still empty — concurrent typing must not be clobbered.
            setKeyInputs((current) => {
              if ((current[provider.id] ?? "") !== "") return current;
              return { ...current, [provider.id]: key };
            });
            setKeyBaselines((current) => {
              // Same empty-guard as keyInputs (ref mirrors latest typed value across the await).
              if ((keyInputsRef.current[provider.id] ?? "") !== "") return current;
              return { ...current, [provider.id]: key };
            });
          }
        } catch {
          // Keep empty when a key cannot be revealed; placeholder still shows configured state.
        } finally {
          keyRevealInFlightRef.current.delete(provider.id);
          setKeyRevealInFlight((current) => {
            const next = { ...current };
            delete next[provider.id];
            return next;
          });
        }
      }
      setKeyVisible((current) => ({ ...current, [provider.id]: true }));
    },
    [keyInputs, keyVisible],
  );

  useEffect(() => {
    let disposed = false;
    void window._popper_
      .getSettings()
      .then((settings) => {
        if (disposed) return;
        const normalized = normalizePublicSettings(settings);
        setDraft(normalized);
        // Do not preload API keys — fetch only when the user clicks show.
        resetProviderKeyFields();
      })
      .catch((error: unknown) => {
        if (!disposed) setBanner({ kind: "error", text: getErrorMessage(error, "无法读取设置") });
      });
    void window._popper_
      .getAccessibilityStatus()
      .then((status) => {
        if (!disposed) setAccessibility(status);
      })
      .catch((error: unknown) => {
        if (!disposed) {
          setBanner({ kind: "error", text: getErrorMessage(error, "无法读取选区状态") });
        }
      });

    const unsubscribe = window._popper_.onSettingsChanged((settings) => {
      if (!dirtyRef.current && !unsavedChangesRef.current) {
        const normalized = normalizePublicSettings(settings);
        setDraft(normalized);
        resetProviderKeyFields();
      }
      runDetached(refreshRuntimeStatus(), {
        scope: "settings",
        operation: "refresh-runtime-status",
      });
    });
    const refreshAccessibility = (): void => {
      runDetached(refreshRuntimeStatus(), {
        scope: "settings",
        operation: "refresh-runtime-status",
      });
    };
    window.addEventListener("focus", refreshAccessibility);
    return () => {
      disposed = true;
      unsubscribe();
      window.removeEventListener("focus", refreshAccessibility);
    };
  }, [resetProviderKeyFields, refreshRuntimeStatus]);

  const changeDraft = useCallback((updater: (current: PublicSettings) => PublicSettings): void => {
    setDraft((current) => (current ? updater(current) : current));
    dirtyRef.current = true;
    setDirty(true);
    setBanner(null);
  }, []);

  const enabledActions = useMemo(
    () => sortAndNumberActions(draft?.actions ?? []).filter((action) => action.enabled),
    [draft?.actions],
  );
  const disabledActions = useMemo(
    () => sortAndNumberActions(draft?.actions ?? []).filter((action) => !action.enabled),
    [draft?.actions],
  );
  const persist = async (): Promise<PublicSettings | null> => {
    if (!draft) return null;
    const validation = buildSettingsUpdate(draft);
    if (!validation.valid) {
      setBanner({ kind: "error", text: validation.message });
      return null;
    }

    let saved: PublicSettings;
    try {
      // The full provider list is part of the ordinary settings transaction.
      // Rust removes orphaned secrets while committing that transaction, so a
      // staged deletion must never run as a separate destructive command first.
      saved = await window._popper_.updateSettings(validation.value);
    } catch (error) {
      await refreshRuntimeStatus().catch(() => undefined);
      setBanner({ kind: "error", text: getErrorMessage(error, "保存设置失败") });
      return null;
    }

    await refreshRuntimeStatus().catch(() => undefined);

    setDraft(normalizePublicSettings(saved));
    setDirty(false);
    dirtyRef.current = false;

    const existingProviderIds = new Set(saved.providers.map((provider) => provider.id));
    const pendingKeyWrites = Object.entries(keyInputs).filter(([providerId, rawKey]) => {
      if (!existingProviderIds.has(providerId)) return false;
      const next = rawKey.trim();
      const baseline = (keyBaselines[providerId] ?? "").trim();
      return next.length > 0 && next !== baseline;
    });

    const remainingInputs = { ...keyInputs };
    for (const [providerId, rawKey] of pendingKeyWrites) {
      const apiKey = rawKey.trim();
      try {
        saved = await window._popper_.setProviderApiKey(providerId, apiKey);
        remainingInputs[providerId] = apiKey;
      } catch (error) {
        const refreshed = await window._popper_.getSettings().catch(() => saved);
        const normalized = normalizePublicSettings(refreshed);
        setDraft(normalized);
        setKeyInputs(remainingInputs);
        setBanner({
          kind: "error",
          text: `普通设置已保存，但 API Key 保存失败：${getErrorMessage(error, "无法保存密钥")}`,
        });
        return null;
      }
    }

    const normalized = normalizePublicSettings(saved);
    setDraft(normalized);
    // Keep keyConfigured from saved settings; do not re-fetch plaintext keys.
    resetProviderKeyFields();
    return saved;
  };

  const save = async (): Promise<void> => {
    setOperation("save");
    setBanner(null);
    const saved = await persist();
    if (saved) setBanner({ kind: "success", text: "设置已保存" });
    setOperation(null);
  };

  const resetResultSize = async (): Promise<void> => {
    setOperation("reset-result-size");
    setBanner(null);
    try {
      if (!window._popper_.resetResultSize) {
        throw new Error("当前后端尚不支持安全重置结果框尺寸");
      }
      await window._popper_.resetResultSize();
      setDraft((current) =>
        current ? { ...current, result: { ...current.result, lastSize: null } } : current,
      );
      setBanner({ kind: "success", text: "已重置结果框尺寸" });
    } catch (error) {
      setBanner({ kind: "error", text: getErrorMessage(error, "无法重置结果框尺寸") });
    } finally {
      setOperation(null);
    }
  };

  const requestAccessibility = async (): Promise<void> => {
    setOperation("accessibility");
    setBanner(null);
    try {
      const status = await window._popper_.requestAccessibility();
      setAccessibility(status);
      setBanner(
        status.trusted
          ? { kind: "success", text: "辅助功能权限已启用" }
          : { kind: "error", text: `请在系统设置中允许 ${APP_NAME}；授权后可能需要重新启动应用` },
      );
    } catch (error) {
      setBanner({ kind: "error", text: getErrorMessage(error, "无法请求辅助功能权限") });
    } finally {
      setOperation(null);
    }
  };

  const addProvider = (): void => {
    const id = createId("provider");
    const provider: PublicProviderSettings = {
      id,
      name: `服务商 ${(draft?.providers.length ?? 0) + 1}`,
      enabled: true,
      kind: "openai",
      baseUrl: DEFAULT_OPENAI_BASE_URL,
      keyConfigured: false,
      models: [],
    };
    changeDraft((current) => ({ ...current, providers: [...current.providers, provider] }));
    setExpandedProviders((current) => ({ ...current, [id]: true }));
  };

  const changeProvider = (
    providerId: string,
    updater: (provider: PublicProviderSettings) => PublicProviderSettings,
  ): void => {
    changeDraft((current) => ({
      ...current,
      providers: current.providers.map((provider) =>
        provider.id === providerId ? updater(provider) : provider,
      ),
    }));
  };

  const changeProviderKind = (providerId: string, kind: "openai" | "dsh"): void => {
    if (draft?.providers.find((provider) => provider.id === providerId)?.kind === kind) return;
    changeDraft((current) => ({
      ...current,
      providers: current.providers.map((provider) => provider.id === providerId ? {
        ...provider, kind,
        baseUrl: kind === "dsh" ? "http://localhost:3080" : DEFAULT_OPENAI_BASE_URL,
        dshWorkspace: kind === "dsh" ? "/workspace" : undefined,
        models: [],
      } : provider),
      defaultAi: current.defaultAi.providerId === providerId ? { providerId: "", modelId: "" } : current.defaultAi,
      actions: current.actions.map((action) => isAiActionDefinition(action) && action.providerId === providerId
        ? { ...action, modelId: "", thinkingMode: "off" as const } : action),
    }));
  };

  const deleteProvider = (provider: PublicProviderSettings): void => {
    changeDraft((current) => removeProviderFromSettings(current, provider.id));
    setExpandedProviders((current) => {
      const next = { ...current };
      delete next[provider.id];
      return next;
    });
    setKeyInputs((current) => {
      const next = { ...current };
      delete next[provider.id];
      return next;
    });
    setKeyBaselines((current) => {
      const next = { ...current };
      delete next[provider.id];
      return next;
    });
    setProviderPendingDelete(null);
    setBanner({ kind: "success", text: `已移除“${provider.name}”；保存设置后生效` });
  };

  const clearProviderKey = async (provider: PublicProviderSettings): Promise<void> => {
    setOperation(`key:${provider.id}`);
    setBanner(null);
    try {
      await window._popper_.clearProviderApiKey(provider.id);
      setDraft((current) =>
        current
          ? {
              ...current,
              providers: current.providers.map((candidate) =>
                candidate.id === provider.id ? { ...candidate, keyConfigured: false } : candidate,
              ),
            }
          : current,
      );
      setKeyInputs((current) => {
        const next = { ...current };
        delete next[provider.id];
        return next;
      });
      setKeyBaselines((current) => {
        const next = { ...current };
        delete next[provider.id];
        return next;
      });
      setKeyVisible((current) => ({ ...current, [provider.id]: false }));
      setBanner({ kind: "success", text: `已清除“${provider.name}”的密钥` });
    } catch (error) {
      setBanner({ kind: "error", text: getErrorMessage(error, "无法清除 API Key") });
    } finally {
      setOperation(null);
    }
  };

  const testProvider = async (provider: PublicProviderSettings): Promise<void> => {
    setOperation(`test:${provider.id}`);
    setBanner(null);
    const saved = await persist();
    if (!saved) {
      setOperation(null);
      return;
    }
    try {
      const result = await window._popper_.testProviderConnection(provider.id);
      if (result.ok) {
        const suffix = result.models.length ? `，发现 ${result.models.length} 个模型` : "";
        setBanner({ kind: "success", text: `“${provider.name}”连接成功${suffix}` });
      } else {
        setBanner({ kind: "error", text: result.message });
      }
    } catch (error) {
      setBanner({ kind: "error", text: getErrorMessage(error, "连接测试失败") });
    } finally {
      setOperation(null);
    }
  };

  const openModelPicker = async (provider: PublicProviderSettings): Promise<void> => {
    setOperation(`fetch:${provider.id}`);
    setBanner(null);
    const saved = await persist();
    if (!saved) {
      setOperation(null);
      return;
    }
    try {
      const result = await window._popper_.listProviderModels(provider.id);
      if (!result.ok) {
        setBanner({ kind: "error", text: result.message });
        return;
      }
      const remoteModels = normalizedModels(result.models);
      const current =
        draft?.providers.find((candidate) => candidate.id === provider.id) ??
        saved.providers.find((candidate) => candidate.id === provider.id);
      if (!current) throw new Error("服务商已不存在");
      setModelPicker({
        providerId: provider.id,
        providerName: provider.name,
        previous: current.models.map((model) => ({ ...model })),
        remoteModels,
        checkedIds: new Set(current.models.map((model) => model.id)),
      });
    } catch (error) {
      setBanner({ kind: "error", text: getErrorMessage(error, "获取模型失败") });
    } finally {
      setOperation(null);
    }
  };

  const applyModelPicker = (checkedIds: ReadonlySet<string>): void => {
    if (!modelPicker) return;
    const { providerId, previous, remoteModels } = modelPicker;
    const nextModels = mergeProviderModelsOnPick({
      previous,
      remote: remoteModels,
      checkedIds,
    });
    const removedIds = previous
      .map((model) => model.id)
      .filter((id) => !nextModels.some((model) => model.id === id));
    changeDraft((current) => {
      let next = current;
      for (const modelId of removedIds) {
        next = removeProviderModelFromSettings(next, providerId, modelId);
      }
      return {
        ...next,
        providers: next.providers.map((provider) =>
          provider.id === providerId ? { ...provider, models: nextModels } : provider,
        ),
      };
    });
    setModelPicker(null);
    setBanner({
      kind: "success",
      text: `已选择 ${nextModels.length} 个模型，请保存设置`,
    });
  };

  const reorderProviderModels = (providerId: string, activeId: string, overId: string): void => {
    if (activeId === overId) return;
    changeProvider(providerId, (provider) => {
      const oldIndex = provider.models.findIndex((model) => model.id === activeId);
      const newIndex = provider.models.findIndex((model) => model.id === overId);
      if (oldIndex < 0 || newIndex < 0) return provider;
      const models = [...provider.models];
      const [moved] = models.splice(oldIndex, 1);
      if (!moved) return provider;
      models.splice(newIndex, 0, moved);
      return { ...provider, models };
    });
  };

  const addManualModel = (provider: PublicProviderSettings): void => {
    const id = (newModelInputs[provider.id] ?? "").trim();
    if (!id) return;
    if (provider.models.some((model) => model.id === id)) {
      setBanner({ kind: "error", text: `模型“${id}”已存在` });
      return;
    }
    changeProvider(provider.id, (current) => ({
      ...current,
      models: [...current.models, { id, name: id, thinkingLevels: [] }],
    }));
    setNewModelInputs((current) => ({ ...current, [provider.id]: "" }));
  };

  const saveAction = (value: ActionEditorValue): void => {
    changeDraft((current) => {
      const existing = editor !== "new" && editor ? editor : null;
      const base = {
        id: existing?.id ?? createId(value.kind),
        name: value.name,
        icon: value.icon,
        kind: value.kind,
        enabled: existing?.enabled ?? false,
        order: existing?.order ?? current.actions.length,
      };
      const action = actionDefinitionSchema.parse({
        ...base,
        providerId: value.providerId ?? "",
        modelId: value.modelId ?? "",
        prompt: value.prompt ?? "",
        thinkingMode: value.thinkingMode ?? "off",
      });
      const actions = existing
        ? current.actions.map((item) => (item.id === existing.id ? action : item))
        : [...current.actions, action];
      return { ...current, actions: sortAndNumberActions(actions) };
    });
    setEditor(null);
  };

  const removeAction = (action: ActionDefinition): void => {
    if (draft && draft.actions.length <= 1) {
      setBanner({ kind: "error", text: "至少需要保留一个动作" });
      return;
    }
    setActionPendingDelete(action);
  };

  const deleteAction = (action: ActionDefinition): void => {
    changeDraft((current) => ({
      ...current,
      actions: sortAndNumberActions(current.actions.filter((item) => item.id !== action.id)),
    }));
    setActionPendingDelete(null);
    setBanner({ kind: "success", text: `已移除动作“${action.name}”；保存设置后生效` });
  };

  const moveToZone = (actionId: string, enabled: boolean, index: number): void => {
    if (!draft) return;
    const action = draft.actions.find((candidate) => candidate.id === actionId);
    if (!action) return;
    if (enabled && !action.enabled && enabledActions.length >= MAX_ENABLED_ACTIONS) {
      setBanner({ kind: "error", text: `工具栏最多显示 ${MAX_ENABLED_ACTIONS} 个动作` });
      return;
    }
    if (!enabled && action.enabled && enabledActions.length <= 1) {
      setBanner({ kind: "error", text: "至少需要启用一个工具栏动作" });
      return;
    }
    changeDraft((current) => ({
      ...current,
      actions: moveActionToZone(current.actions, actionId, enabled, index),
    }));
  };

  const handleDragStart = (event: DragStartEvent): void => {
    setDraggingId(String(event.active.id));
  };

  const handleDragEnd = (event: DragEndEvent): void => {
    setDraggingId(null);
    if (!draft || !event.over) return;

    const actionId = String(event.active.id);
    const overId = String(event.over.id);
    if (overId === ACTION_ZONE_IDS.enabled || overId === ACTION_ZONE_IDS.disabled) {
      const enabled = overId === ACTION_ZONE_IDS.enabled;
      const targetLength = enabled ? enabledActions.length : disabledActions.length;
      moveToZone(actionId, enabled, targetLength);
      return;
    }

    const overAction = draft.actions.find((action) => action.id === overId);
    if (!overAction) return;
    const targetActions = overAction.enabled ? enabledActions : disabledActions;
    const targetIndex = targetActions.findIndex((action) => action.id === overAction.id);
    if (targetIndex >= 0) moveToZone(actionId, overAction.enabled, targetIndex);
  };

  if (!draft) {
    return (
      <main className="settings-loading">
        <LoaderCircle className="settings-spin" size={24} aria-hidden="true" />
        <span>正在载入设置…</span>
        {banner?.kind === "error" && <div className="notice notice--error">{banner.text}</div>}
      </main>
    );
  }

  const busy = operation !== null;
  const permission = accessibilityPresentation(accessibility);
  const selectionAvailable = accessibility?.available ?? accessibility?.trusted ?? false;

  const toggleSelectionDebug = async (enabled: boolean): Promise<void> => {
    const update = window._popper_.setSelectionDetectionDebugEnabled;
    if (!update || debugUpdating) return;
    setDebugUpdating(true);
    try {
      selectionDebug.setEnabled(await update(enabled));
    } catch (error) {
      setBanner({ kind: "error", text: `切换 DEV Debug 失败：${getErrorMessage(error)}` });
    } finally {
      setDebugUpdating(false);
    }
  };

  return (
    <main className="settings-page settings-page--shell">
      <aside className="settings-shell__nav">
        <header className="settings-hero">
          <img className="settings-logo" src={appIcon} alt="" aria-hidden="true" />
          <div>
            <h1>{APP_NAME}</h1>
            <p>让每一次划词更顺手</p>
          </div>
        </header>
        <span className="settings-nav__label">偏好设置</span>
        <nav className="settings-nav" aria-label="设置分区">
          {SETTINGS_SECTIONS.map((section) => (
            <button
              key={section.id}
              type="button"
              className={`settings-nav__item ${activeSection === section.id ? "is-selected" : ""}`}
              aria-current={activeSection === section.id ? "page" : undefined}
              aria-label={section.label}
              onClick={() => setActiveSection(section.id)}
            >
              <section.icon size={19} aria-hidden="true" />
              <span className="settings-nav__copy">
                <span>{section.label}</span>
                <small>{section.caption}</small>
              </span>
            </button>
          ))}
        </nav>
        <div className="settings-sidebar-controls">
          <label className="settings-debug-toggle settings-tray-toggle">
            <span className="settings-debug-toggle__copy">
              <strong>启用托盘</strong>
              <small>再次运行 Popper 可打开设置</small>
            </span>
            <span className="switch switch--small">
              <input
                type="checkbox"
                role="switch"
                aria-label="启用托盘"
                checked={draft?.trayEnabled ?? true}
                disabled={!draft || operation !== null}
                onChange={(event) => {
                  const enabled = event.currentTarget.checked;
                  changeDraft((current) => ({ ...current, trayEnabled: enabled }));
                }}
              />
              <span aria-hidden="true" />
            </span>
          </label>
          {accessibility?.platform === "windows" && (
            <div className="settings-debug">
              <label className="settings-debug-toggle" title="记录选区及附近文本，并在结果窗口显示诊断报告">
                <span className="settings-debug-toggle__copy">
                  <strong>DEV Debug</strong>
                  <small>选区诊断 · 重启后关闭</small>
                </span>
                <span className="switch switch--small">
                  <input
                    type="checkbox"
                    role="switch"
                    aria-label="DEV Debug"
                    checked={selectionDebug.enabled}
                    disabled={!selectionDebug.ready || debugUpdating}
                    onChange={(event) => void toggleSelectionDebug(event.currentTarget.checked)}
                  />
                  <span aria-hidden="true" />
                </span>
              </label>
            </div>
          )}
        </div>
      </aside>

      <div className="settings-shell__main">
        {banner && (
          <div
            className={`notice notice--${banner.kind} settings-banner ${
              bannerFading ? "settings-banner--fading" : ""
            }`}
            role={banner.kind === "error" ? "alert" : "status"}
          >
            {banner.kind === "success" ? (
              <Check size={16} aria-hidden="true" />
            ) : (
              <CircleAlert size={16} aria-hidden="true" />
            )}
            <span>{banner.text}</span>
          </div>
        )}

        <div className="settings-shell__content" key={activeSection}>
          {activeSection === "general" && (
            <>
              <section className="settings-section settings-section--enter" aria-labelledby="providers-title">
                <div className="section-heading section-heading--actions">
                  <div>
                    <h2 id="providers-title">AI 服务商与模型</h2>
                    <p>配置 API 地址和密钥，获取可用模型。</p>
                  </div>
                  <button className="button" type="button" onClick={addProvider}>
                    <Plus size={15} />
                    添加服务商
                  </button>
                </div>
                <div className="provider-stack">
                  {draft.providers.map((provider) => (
                    <article
                      className={`settings-card provider-card ${provider.enabled === false ? "provider-card--disabled" : ""}`}
                      key={provider.id}
                    >
                      <header
                        className={`provider-card__header ${expandedProviders[provider.id] ? "is-expanded" : ""}`}
                      >
                        <div className="provider-card__identity">
                          <button
                            className="icon-button provider-collapse-toggle"
                            type="button"
                            aria-label={`${expandedProviders[provider.id] ? "收起" : "展开"}${provider.name}配置`}
                            aria-expanded={Boolean(expandedProviders[provider.id])}
                            aria-controls={`provider-config-${provider.id}`}
                            onClick={() =>
                              setExpandedProviders((current) => ({
                                ...current,
                                [provider.id]: !current[provider.id],
                              }))
                            }
                          >
                            {expandedProviders[provider.id] ? (
                              <ChevronDown size={16} />
                            ) : (
                              <ChevronRight size={16} />
                            )}
                          </button>
                          <span className="provider-avatar">
                            <Sparkles size={17} />
                          </span>
                          <div className="provider-card__name">
                            <input
                              className="provider-name-input"
                              value={provider.name}
                              maxLength={80}
                              aria-label="服务商名称"
                              onChange={(event) =>
                                changeProvider(provider.id, (current) => ({
                                  ...current,
                                  name: event.target.value,
                                }))
                              }
                            />
                            <span className="provider-card__summary">
                              {provider.models.length} 个模型 · {provider.keyConfigured ? "密钥已保存" : !requiresProviderApiKey(provider) ? "本地自动登录" : "未保存密钥"}
                            </span>
                          </div>
                        </div>
                        <div className="provider-card__header-actions">
                          <label
                            className="provider-enabled-toggle"
                            title={
                              provider.enabled === false
                                ? "已关闭：动作与结果中不显示其模型"
                                : "已启用"
                            }
                          >
                            <span className="provider-enabled-toggle__label">
                              {provider.enabled === false ? "已关闭" : "已启用"}
                            </span>
                            <input
                              type="checkbox"
                              role="switch"
                              aria-label={`${provider.name}：${provider.enabled === false ? "启用" : "关闭"}服务商`}
                              checked={provider.enabled !== false}
                              onChange={(event) =>
                                changeProvider(provider.id, (current) => ({
                                  ...current,
                                  enabled: event.target.checked,
                                }))
                              }
                            />
                          </label>
                          <button
                            className="icon-button action-delete"
                            type="button"
                            title="删除服务商"
                            aria-label={`删除${provider.name}`}
                            onClick={() => setProviderPendingDelete(provider)}
                          >
                            <Trash2 size={16} />
                          </button>
                        </div>
                      </header>
                      <div
                        className="provider-grid"
                        id={`provider-config-${provider.id}`}
                        hidden={!expandedProviders[provider.id]}
                      >
                        <div className="field field--wide">
                          <span className="field__label">接入方式</span>
                          <StyledSelect
                            aria-label={`${provider.name}接入方式`}
                            value={provider.kind}
                            disabled={busy}
                            options={[
                              { value: "openai", label: "OpenAI 兼容接口" },
                              { value: "dsh", label: "DSH Docker" },
                            ]}
                            onChange={(value) => changeProviderKind(provider.id, value as "openai" | "dsh")}
                          />
                        </div>
                        {provider.kind === "dsh" && (
                          <label className="field field--wide">
                            <span className="field__label">DSH 工作区</span>
                            <input className="control" aria-label="DSH 工作区" value={provider.dshWorkspace ?? "/workspace"}
                              placeholder="/workspace" spellCheck={false}
                              onChange={(event) => changeProvider(provider.id, (current) => ({
                                ...current, dshWorkspace: event.target.value,
                              }))} />
                            <span className="field__hint">
                              填写容器内路径。工具审批与问题在同一实例的 DSH 网页中处理；Popper 会提示你前往处理。
                            </span>
                          </label>
                        )}
                        <label className="field field--wide">
                          <span className="field__label">API 地址（HTTP / HTTPS）</span>
                          <input
                            className="control"
                            type="url"
                            value={provider.baseUrl}
                            spellCheck={false}
                            placeholder={provider.kind === "dsh" ? "http://localhost:3080" : "https://api.example.com/v1 或 http://localhost:11434/v1"}
                            onChange={(event) =>
                              changeProvider(provider.id, (current) => ({
                                ...current,
                                baseUrl: event.target.value,
                              }))
                            }
                          />
                          <span className="field__hint">
                            {provider.kind === "dsh" ? (
                              <>填写 DSH Docker 网关地址，例如 <code>http://localhost:3080</code>，不带 <code>/api</code>。localhost 可留空 API Key，Popper 会自动登录；远程访问填写 <code>DSH_API_KEY</code>。模型凭据在 DSH 网页中配置。</>
                            ) : (
                              <>填写到版本前缀即可，例如 <code>…/v1</code>；不要带 <code>/chat/completions</code>。本地 Ollama 可用 HTTP。</>
                            )}
                          </span>
                          {usesPlainHttp(provider.baseUrl) && (requiresProviderApiKey(provider) || provider.keyConfigured || Boolean(keyInputs[provider.id]?.trim())) && (
                            <span className="provider-http-warning" role="alert">
                              <CircleAlert size={15} aria-hidden="true" />
                              当前服务使用 HTTP，API Key
                              将通过网络明文传输。仅在你信任的网络和服务中使用。
                            </span>
                          )}
                        </label>
                        <label className="field field--wide">
                          <span className="field__label">{requiresProviderApiKey(provider) ? "API Key" : "API Key（本地可留空）"}</span>
                          <div className="key-control">
                            <KeyRound size={16} aria-hidden="true" />
                            <input
                              type={keyVisible[provider.id] ? "text" : "password"}
                              value={keyInputs[provider.id] ?? ""}
                              maxLength={16_384}
                              autoComplete="new-password"
                              spellCheck={false}
                              placeholder={
                                provider.keyConfigured || (keyBaselines[provider.id] ?? "")
                                  ? "已保存的 API Key"
                                  : requiresProviderApiKey(provider) ? "输入 API Key" : "本地自动登录，无需填写"
                              }
                              onChange={(event) => {
                                const value = event.target.value;
                                // Keep ref in sync before re-render so in-flight reveal won't clobber baseline checks.
                                keyInputsRef.current = {
                                  ...keyInputsRef.current,
                                  [provider.id]: value,
                                };
                                setKeyInputs((current) => ({ ...current, [provider.id]: value }));
                              }}
                            />
                            <button
                              className="key-visibility-toggle"
                              type="button"
                              disabled={Boolean(keyRevealInFlight[provider.id])}
                              aria-label={keyVisible[provider.id] ? "隐藏 API Key" : "显示 API Key"}
                              title={keyVisible[provider.id] ? "隐藏" : "显示"}
                              onClick={() => void revealProviderApiKey(provider)}
                            >
                              {keyVisible[provider.id] ? (
                                <EyeOff size={16} aria-hidden="true" />
                              ) : (
                                <Eye size={16} aria-hidden="true" />
                              )}
                            </button>
                          </div>
                        </label>
                        <div className="provider-actions">
                          <button
                            className="button"
                            type="button"
                            disabled={busy}
                            onClick={() => void testProvider(provider)}
                          >
                            {operation === `test:${provider.id}` && (
                              <LoaderCircle className="settings-spin" size={15} />
                            )}
                            测试连接
                          </button>
                          <button
                            className="button"
                            type="button"
                            disabled={busy}
                            onClick={() => void openModelPicker(provider)}
                          >
                            {operation === `fetch:${provider.id}` ? (
                              <LoaderCircle className="settings-spin" size={15} />
                            ) : (
                              <RefreshCw size={15} />
                            )}
                            获取模型
                          </button>
                          <button
                            className="button button--danger"
                            type="button"
                            disabled={busy || !provider.keyConfigured}
                            onClick={() => void clearProviderKey(provider)}
                          >
                            清除密钥
                          </button>
                        </div>
                        <div className="field field--wide">
                          <span className="field__label">模型（{provider.models.length}）</span>
                          <div className="model-add-row">
                            <input
                              className="control"
                              value={newModelInputs[provider.id] ?? ""}
                              placeholder={provider.kind === "dsh" ? "DSH 提供商 ID/模型 ID" : "手动添加模型 ID"}
                              spellCheck={false}
                              onChange={(event) =>
                                setNewModelInputs((current) => ({
                                  ...current,
                                  [provider.id]: event.target.value,
                                }))
                              }
                              onKeyDown={(event) => {
                                if (event.key === "Enter") {
                                  event.preventDefault();
                                  addManualModel(provider);
                                }
                              }}
                            />
                            <button
                              className="button"
                              type="button"
                              onClick={() => addManualModel(provider)}
                            >
                              添加
                            </button>
                          </div>
                          <DndContext
                            sensors={dragSensors}
                            collisionDetection={closestCenter}
                            onDragStart={(event) => setDraggingModelId(String(event.active.id))}
                            onDragCancel={() => setDraggingModelId(null)}
                            onDragEnd={(event) => {
                              setDraggingModelId(null);
                              const { active, over } = event;
                              if (!over) return;
                              reorderProviderModels(provider.id, String(active.id), String(over.id));
                            }}
                          >
                            <SortableContext
                              items={provider.models.map((model) => model.id)}
                              strategy={verticalListSortingStrategy}
                            >
                              <div className="model-row-list" role="list">
                                {provider.models.map((model) => (
                                  <SortableModelRow
                                    key={model.id}
                                    model={model}
                                    dragging={draggingModelId === model.id}
                                    onRemove={() =>
                                      changeDraft((current) =>
                                        removeProviderModelFromSettings(
                                          current,
                                          provider.id,
                                          model.id,
                                        ),
                                      )
                                    }
                                  />
                                ))}
                                {provider.models.length === 0 && (
                                  <span className="model-empty">
                                    尚无模型。可点「获取模型」多选添加，或在上方手动输入模型 ID。
                                  </span>
                                )}
                              </div>
                            </SortableContext>
                          </DndContext>
                        </div>
                      </div>
                    </article>
                  ))}
                  {draft.providers.length === 0 && (
                    <div className="settings-card empty-card empty-card--guided">
                      <strong>尚未配置服务商</strong>
                      <p>
                        使用翻译、总结等 AI 功能时，点右上角「添加服务商」开始。
                      </p>
                    </div>
                  )}
                </div>
              </section>

              <section className="settings-section" aria-labelledby="permission-title">
                <div className="section-heading">
                  <div>
                    <h2 id="permission-title">{permission.heading}</h2>
                    <p>{permission.description}</p>
                  </div>
                </div>
                <div className="settings-card permission-card">
                  <div
                    className={`permission-icon ${selectionAvailable ? "permission-icon--ok" : ""}`}
                  >
                    {selectionAvailable ? <ShieldCheck size={21} /> : <LockKeyhole size={21} />}
                  </div>
                  <div className="permission-copy">
                    <strong>{permission.state}</strong>
                    <p>{permission.detail}</p>
                  </div>
                  <button
                    className="button"
                    type="button"
                    disabled={busy || accessibility?.trusted || !accessibility?.canRequest}
                    onClick={() => void requestAccessibility()}
                  >
                    {operation === "accessibility" && (
                      <LoaderCircle className="settings-spin" size={15} />
                    )}
                    {permission.button}
                  </button>
                </div>
              </section>
            </>
          )}

          {activeSection === "actions" && (
            <section
              className="settings-section settings-section--enter"
              aria-label="动作设置"
            >
              <div className="settings-group" id="default-ai-title">
                <div className="section-heading section-heading--actions default-ai-heading">
                  <div>
                    <h3 className="settings-subheading">默认模型</h3>
                    <p>用于快捷提问和划词提问；动作中单独指定的模型优先。</p>
                  </div>
                  <StyledSelect
                    className="default-ai-select"
                    aria-label="默认模型"
                    disabled={busy}
                    value={draft.defaultAi.modelId ? JSON.stringify([draft.defaultAi.providerId, draft.defaultAi.modelId]) : ""}
                    onChange={(value) => {
                      const [providerId, modelId] = value ? JSON.parse(value) as [string, string] : ["", ""];
                      changeDraft((current) => ({ ...current, defaultAi: { providerId, modelId } }));
                    }}
                    options={[
                      { value: "", label: "选择默认模型" },
                      ...draft.providers.filter((provider) => provider.enabled !== false).flatMap((provider) =>
                        provider.models.map((model) => ({
                          value: JSON.stringify([provider.id, model.id]),
                          label: model.name,
                          group: provider.keyConfigured || !requiresProviderApiKey(provider) ? provider.name : provider.name + "（未配置密钥）",
                        })),
                      ),
                    ]}
                  />
                </div>
              </div>

              <div className="settings-group" id="quick-ask-title">
                <div className="section-heading section-heading--compact">
                  <div>
                    <h3 className="settings-subheading">快捷提问</h3>
                    <p>无需划词，随时向 AI 提问；支持添加图片、文档和文本附件。</p>
                  </div>
                </div>
                <div className="settings-card quick-ask-setting">
                  <div className="quick-ask-setting__label">
                    <strong>唤出快捷键</strong>
                    <span>修改后点击「保存设置」生效。</span>
                  </div>
                  <div className="quick-ask-setting__controls">
                    <ShortcutRecorder
                      value={draft.quickAskShortcut}
                      disabled={busy}
                      onChange={(shortcut) => changeDraft((current) => ({ ...current, quickAskShortcut: shortcut }))}
                    />
                    <button
                      className="button button--quiet quick-ask-setting__reset"
                      type="button"
                      disabled={busy || draft.quickAskShortcut === "Control+Shift+Space"}
                      onClick={() => changeDraft((current) => ({ ...current, quickAskShortcut: "Control+Shift+Space" }))}
                    >
                      <RefreshCw size={13} aria-hidden="true" />
                      恢复默认
                    </button>
                  </div>
                </div>
                {accessibility?.diagnostics?.quickAskShortcutError && (
                  <p className="field-error" role="alert">
                    {accessibility.diagnostics.quickAskShortcutError}，请更换组合键并保存。
                  </p>
                )}
              </div>

              <div className="section-heading section-heading--compact selection-ask-heading" id="selection-ask-title">
                <div>
                  <h3 className="settings-subheading">划词提问</h3>
                  <p>
                    选中文字后长按右键 <kbd>250 ms</kbd> 唤出工具栏，短按仍是普通右键。
                  </p>
                </div>
              </div>

              <div className="selection-subnav" role="tablist" aria-label="动作配置选项卡">
                <button
                  type="button"
                  role="tab"
                  id="selection-tab-actions"
                  aria-selected={selectionSubTab === "actions"}
                  aria-controls="selection-panel-actions"
                  className={`selection-subnav__item ${selectionSubTab === "actions" ? "is-selected" : ""}`}
                  onClick={() => handleSelectSubTab("actions")}
                >
                  <WandSparkles size={15} aria-hidden="true" />
                  <span className="selection-subnav__label">划词动作</span>
                  <span className="selection-subnav__badge">
                    {enabledActions.length} / {MAX_ENABLED_ACTIONS}
                  </span>
                </button>
                <button
                  type="button"
                  role="tab"
                  id="selection-tab-result"
                  aria-selected={selectionSubTab === "result"}
                  aria-controls="selection-panel-result"
                  className={`selection-subnav__item ${selectionSubTab === "result" ? "is-selected" : ""}`}
                  onClick={() => handleSelectSubTab("result")}
                >
                  <Monitor size={15} aria-hidden="true" />
                  <span className="selection-subnav__label">结果窗口</span>
                  <span className="selection-subnav__badge">
                    {draft.result.dismissMode === "blur"
                      ? "失焦关闭"
                      : draft.result.dismissMode === "pointer-leave"
                        ? "移出关闭"
                        : "手动关闭"}
                  </span>
                </button>
                <button
                  type="button"
                  role="tab"
                  id="selection-tab-filter"
                  aria-selected={selectionSubTab === "filter"}
                  aria-controls="selection-panel-filter"
                  className={`selection-subnav__item ${selectionSubTab === "filter" ? "is-selected" : ""}`}
                  onClick={() => handleSelectSubTab("filter")}
                >
                  <ListFilter size={15} aria-hidden="true" />
                  <span className="selection-subnav__label">应用过滤</span>
                  <span className="selection-subnav__badge">
                    {draft.filter.mode === "default"
                      ? "全部应用"
                      : draft.filter.mode === "whitelist"
                        ? `白名单 (${draft.filter.applications.length})`
                        : `黑名单 (${draft.filter.applications.length})`}
                  </span>
                </button>
              </div>

              {selectionSubTab === "actions" && (
                <div
                  key="selection-panel-actions"
                  id="selection-panel-actions"
                  role="tabpanel"
                  aria-labelledby="selection-tab-actions"
                  className="selection-subpanel selection-subpanel--enter"
                >
                  <div className="selection-actions-bar" id="actions-title">
                    <p className="selection-actions-bar__tip">
                      拖拽或勾选可调整在划词工具栏中出现的动作及排序，最多启用 {MAX_ENABLED_ACTIONS} 个。
                    </p>
                    <button className="button" type="button" onClick={() => setEditor("new")}>
                      <Plus size={15} />
                      添加动作
                    </button>
                  </div>
                  <div className="settings-card toolbar-preview-card">
                    <span className="toolbar-preview-label">实时预览</span>
                    <div className="toolbar-preview" role="toolbar" aria-label="工具栏实时预览">
                      <div className="toolbar-pill">
                        <ToolbarActions actions={getToolbarActions(draft.actions)} preview />
                      </div>
                    </div>
                    <p className="toolbar-preview-note">
                      问 AI 是固定在工具栏最前方的内置功能，不计入动作数量。
                    </p>
                  </div>
                  <DndContext
                    sensors={dragSensors}
                    collisionDetection={closestCenter}
                    onDragStart={handleDragStart}
                    onDragCancel={() => setDraggingId(null)}
                    onDragEnd={handleDragEnd}
                  >
                    <div className="action-zones">
                      <ActionZone
                        title="显示在工具栏"
                        count={`${enabledActions.length} / ${MAX_ENABLED_ACTIONS}`}
                        actions={enabledActions}
                        enabled
                        draggingId={draggingId}
                        onEdit={setEditor}
                        onDelete={removeAction}
                        onToggle={(action) => moveToZone(action.id, false, disabledActions.length)}
                      />
                      <ActionZone
                        title="未显示"
                        count={`${disabledActions.length}`}
                        actions={disabledActions}
                        enabled={false}
                        draggingId={draggingId}
                        onEdit={setEditor}
                        onDelete={removeAction}
                        onToggle={(action) => moveToZone(action.id, true, enabledActions.length)}
                      />
                    </div>
                  </DndContext>
                </div>
              )}

              {selectionSubTab === "result" && (
                <div
                  key="selection-panel-result"
                  id="selection-panel-result"
                  role="tabpanel"
                  aria-labelledby="selection-tab-result"
                  className="selection-subpanel selection-subpanel--enter"
                >
                  <div className="settings-group" id="result-title">
                    <h4 className="settings-group__title">位置与尺寸</h4>
                    <div className="settings-card settings-card--rows">
                      <SettingSwitch
                        title="跟随鼠标位置"
                        description="在点击动作时的鼠标附近打开结果，空间不足时自动翻转。"
                        checked={draft.result.followCursor}
                        onChange={(followCursor) =>
                          changeDraft((current) => ({
                            ...current,
                            result: { ...current.result, followCursor },
                          }))
                        }
                      />
                      <SettingSwitch
                        title="记住结果框尺寸"
                        description={
                          draft.result.lastSize
                            ? `当前记忆：${Math.round(draft.result.lastSize.width)} × ${Math.round(draft.result.lastSize.height)}`
                            : "用户调整后，下次沿用相同逻辑尺寸。"
                        }
                        checked={draft.result.rememberSize}
                        onChange={(rememberSize) =>
                          changeDraft((current) => ({
                            ...current,
                            result: { ...current.result, rememberSize },
                          }))
                        }
                        extra={
                          draft.result.lastSize ? (
                            <button
                              className="button button--quiet"
                              type="button"
                              disabled={busy}
                              onClick={() => void resetResultSize()}
                            >
                              重置尺寸
                            </button>
                          ) : undefined
                        }
                      />
                      <SettingSwitch
                        title="默认置顶"
                        description="新打开的结果窗口保持在其他窗口上方；也可在窗口标题栏临时切换。"
                        checked={draft.result.defaultPinned}
                        onChange={(defaultPinned) =>
                          changeDraft((current) => ({
                            ...current,
                            result: { ...current.result, defaultPinned },
                          }))
                        }
                      />
                    </div>
                  </div>

                  <div className="settings-group">
                    <h4 className="settings-group__title">阅读与排版</h4>
                    <div className="settings-card settings-card--rows">
                      <div className="setting-row">
                        <div>
                          <strong>结果文字大小</strong>
                          <p>调整翻译、总结、解释等结果正文的字号。</p>
                        </div>
                        <div className="range-control">
                          <input
                            type="range"
                            aria-label="结果文字大小"
                            min={RESULT_FONT_SIZE_MIN}
                            max={RESULT_FONT_SIZE_MAX}
                            step={1}
                            value={draft.result.fontSize}
                            onChange={(event) =>
                              changeDraft((current) => ({
                                ...current,
                                result: { ...current.result, fontSize: Number(event.target.value) },
                              }))
                            }
                          />
                          <span>{draft.result.fontSize} px</span>
                        </div>
                      </div>
                      <div className="setting-row">
                        <div>
                          <strong>窗口透明度</strong>
                          <p>内容和控件会保持统一透明度。</p>
                        </div>
                        <div className="range-control">
                          <input
                            type="range"
                            min={20}
                            max={100}
                            value={Math.round(draft.result.opacity * 100)}
                            onChange={(event) =>
                              changeDraft((current) => ({
                                ...current,
                                result: { ...current.result, opacity: Number(event.target.value) / 100 },
                              }))
                            }
                          />
                          <span>{Math.round(draft.result.opacity * 100)}%</span>
                        </div>
                      </div>
                    </div>
                  </div>

                  <div className="settings-group">
                    <h4 className="settings-group__title">关闭规则</h4>
                    <div className="settings-card settings-card--rows">
                      <div className="setting-row">
                        <div>
                          <strong>关闭方式</strong>
                          <p>仅未置顶窗口执行；置顶窗口始终忽略自动关闭。</p>
                        </div>
                        <StyledSelect
                          className="result-mode-control"
                          aria-label="关闭方式"
                          value={draft.result.dismissMode}
                          onChange={(val) =>
                            changeDraft((current) => ({
                              ...current,
                              result: {
                                ...current.result,
                                dismissMode: val as PublicSettings["result"]["dismissMode"],
                              },
                            }))
                          }
                          options={[
                            { value: "blur", label: "失去焦点时关闭（推荐）" },
                            { value: "pointer-leave", label: "鼠标移出后关闭" },
                            { value: "manual", label: "手动关闭" },
                          ]}
                        />
                      </div>
                      {draft.result.dismissMode === "pointer-leave" && (
                        <div className="setting-row">
                          <div>
                            <strong>移出延迟</strong>
                            <p>短暂移出不会误关窗口。</p>
                          </div>
                          <div className="range-control">
                            <input
                              type="range"
                              min={100}
                              max={2000}
                              step={50}
                              value={draft.result.dismissDelayMs}
                              onChange={(event) =>
                                changeDraft((current) => ({
                                  ...current,
                                  result: {
                                    ...current.result,
                                    dismissDelayMs: Number(event.target.value),
                                  },
                                }))
                              }
                            />
                            <span>{draft.result.dismissDelayMs} ms</span>
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )}

              {selectionSubTab === "filter" && (
                <div
                  key="selection-panel-filter"
                  id="selection-panel-filter"
                  role="tabpanel"
                  aria-labelledby="selection-tab-filter"
                  className="selection-subpanel selection-subpanel--enter"
                >
                  <div className="settings-group" id="filter-title">
                    <div className="settings-card filter-panel-card">
                      <div className="filter-mode-header">
                        <div>
                          <strong>过滤模式</strong>
                          <p>选择划词工具栏在不同应用程序中的响应规则。</p>
                        </div>
                      </div>

                      <div className="filter-mode-grid" role="radiogroup" aria-label="过滤策略选择">
                        <button
                          type="button"
                          role="radio"
                          aria-checked={draft.filter.mode === "default"}
                          className={`filter-card-option ${draft.filter.mode === "default" ? "is-selected" : ""}`}
                          onClick={() =>
                            changeDraft((current) => ({
                              ...current,
                              filter: { ...current.filter, mode: "default" },
                            }))
                          }
                        >
                          <div className="filter-card-option__icon">
                            <Globe size={18} aria-hidden="true" />
                          </div>
                          <div className="filter-card-option__body">
                            <span className="filter-card-option__title">所有应用</span>
                            <span className="filter-card-option__desc">
                              在所有支持的软件中启用划词工具栏，无需规则维护
                            </span>
                          </div>
                          <div className="filter-card-option__indicator" aria-hidden="true">
                            {draft.filter.mode === "default" && <Check size={14} />}
                          </div>
                        </button>

                        <button
                          type="button"
                          role="radio"
                          aria-checked={draft.filter.mode === "blacklist"}
                          className={`filter-card-option ${draft.filter.mode === "blacklist" ? "is-selected" : ""}`}
                          onClick={() =>
                            changeDraft((current) => ({
                              ...current,
                              filter: { ...current.filter, mode: "blacklist" },
                            }))
                          }
                        >
                          <div className="filter-card-option__icon filter-card-option__icon--danger">
                            <ShieldBan size={18} aria-hidden="true" />
                          </div>
                          <div className="filter-card-option__body">
                            <span className="filter-card-option__title">排除列表（黑名单）</span>
                            <span className="filter-card-option__desc">
                              在下方指定的应用中静默禁用，其余软件中正常触发
                            </span>
                          </div>
                          <div className="filter-card-option__indicator" aria-hidden="true">
                            {draft.filter.mode === "blacklist" && <Check size={14} />}
                          </div>
                        </button>

                        <button
                          type="button"
                          role="radio"
                          aria-checked={draft.filter.mode === "whitelist"}
                          className={`filter-card-option ${draft.filter.mode === "whitelist" ? "is-selected" : ""}`}
                          onClick={() =>
                            changeDraft((current) => ({
                              ...current,
                              filter: { ...current.filter, mode: "whitelist" },
                            }))
                          }
                        >
                          <div className="filter-card-option__icon filter-card-option__icon--success">
                            <ShieldCheck size={18} aria-hidden="true" />
                          </div>
                          <div className="filter-card-option__body">
                            <span className="filter-card-option__title">仅列表应用（白名单）</span>
                            <span className="filter-card-option__desc">
                              仅在下方指定的软件中响应划词，避免意外打扰
                            </span>
                          </div>
                          <div className="filter-card-option__indicator" aria-hidden="true">
                            {draft.filter.mode === "whitelist" && <Check size={14} />}
                          </div>
                        </button>
                      </div>

                      {draft.filter.mode === "default" ? (
                        <div className="filter-mode-hint filter-mode-hint--banner">
                          <Sparkles size={16} className="filter-mode-hint__icon" aria-hidden="true" />
                          <p>划词工具栏将在所有应用中响应，无需额外配置排除或白名单规则。</p>
                        </div>
                      ) : (
                        <div className="filter-list-editor">
                          <div className="filter-list-editor__header">
                            <div className="filter-list-editor__title">
                              <strong>
                                {draft.filter.mode === "whitelist" ? "白名单应用列表" : "排除应用列表"}
                              </strong>
                              <span className="filter-count-badge">
                                已配置 {draft.filter.applications.length} 项
                              </span>
                            </div>
                            {draft.filter.applications.length > 0 && (
                              <button
                                type="button"
                                className="button button--ghost button--tiny"
                                onClick={() =>
                                  changeDraft((current) => ({
                                    ...current,
                                    filter: { ...current.filter, applications: [] },
                                  }))
                                }
                              >
                                <Trash2 size={13} aria-hidden="true" />
                                清空列表
                              </button>
                            )}
                          </div>

                          <textarea
                            className="control filter-list-input"
                            aria-label="应用过滤列表"
                            placeholder={"每行填写一个应用名称、Bundle ID 或可执行文件名，例如：\nCode\nGoogle Chrome\ncom.apple.Terminal\nnotepad.exe"}
                            value={draft.filter.applications.join("\n")}
                            onChange={(event) =>
                              changeDraft((current) => ({
                                ...current,
                                filter: {
                                  ...current.filter,
                                  applications: event.target.value
                                    .split(/\r?\n/u)
                                    .map((line) => line.trim())
                                    .filter(Boolean),
                                },
                              }))
                            }
                          />

                          <div className="filter-list-tools">
                            <div className="filter-list-presets">
                              <span className="filter-list-presets__label">快速添加常用：</span>
                              {["Code", "Google Chrome", "com.apple.Terminal", "notepad.exe"].map((preset) => (
                                <button
                                  key={preset}
                                  type="button"
                                  className="filter-preset-chip"
                                  disabled={draft.filter.applications.includes(preset)}
                                  onClick={() =>
                                    changeDraft((current) => ({
                                      ...current,
                                      filter: {
                                        ...current.filter,
                                        applications: current.filter.applications.includes(preset)
                                          ? current.filter.applications
                                          : [...current.filter.applications, preset],
                                      },
                                    }))
                                  }
                                >
                                  + {preset}
                                </button>
                              ))}
                            </div>
                            <p className="filter-list-guidance">
                              提示：支持按行输入应用程序名（如 Google Chrome）、Bundle ID（如 com.apple.Terminal）或可执行文件名（如 Code.exe）。
                            </p>
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )}
            </section>
          )}

          {activeSection === "screen" && (
            <section
              className="settings-section settings-section--enter"
              aria-label="屏幕设置"
            >
              {accessibility && accessibility.platform !== "windows" ? (
                <div className="settings-card empty-card empty-card--guided">
                  <strong>屏幕捕捉目前仅支持 Windows 平台</strong>
                  <p>当前系统暂不支持屏幕截图、标注与浮动贴图功能。</p>
                </div>
              ) : (
                <div className="settings-group" id="capture-title">
                  <div className="section-heading section-heading--compact">
                    <div>
                      <h3 className="settings-subheading">截图与贴图</h3>
                      <p>捕捉鼠标所在屏幕，支持框选、标注、浮动贴图和图片提问。贴图在退出后释放。</p>
                    </div>
                  </div>
                  {([
                    ["shortcut", "截图快捷键"],
                    ["pasteShortcut", "贴图快捷键"],
                  ] as const).map(([field, label]) => (
                    <div className="settings-card quick-ask-setting" key={field}>
                      <div className="quick-ask-setting__label">
                        <strong>{label}</strong>
                        <span>修改后点击「保存设置」生效。</span>
                      </div>
                      <div className="quick-ask-setting__controls">
                        <ShortcutRecorder
                          id={"capture-" + field}
                          label={label}
                          value={draft.capture[field]}
                          disabled={busy}
                          onChange={(shortcut) =>
                            changeDraft((current) => ({
                              ...current,
                              capture: { ...current.capture, [field]: shortcut },
                            }))
                          }
                        />
                        <button
                          className="button button--quiet quick-ask-setting__reset"
                          type="button"
                          aria-label={"恢复" + label + "默认"}
                          disabled={busy || draft.capture[field] === DEFAULT_APP_SETTINGS.capture[field]}
                          onClick={() =>
                            changeDraft((current) => ({
                              ...current,
                              capture: { ...current.capture, [field]: DEFAULT_APP_SETTINGS.capture[field] },
                            }))
                          }
                        >
                          <RefreshCw size={13} aria-hidden="true" />
                          恢复默认
                        </button>
                      </div>
                    </div>
                  ))}
                  <div className="settings-card quick-ask-setting">
                    <div className="quick-ask-setting__label">
                      <strong>标注工具</strong>
                      <span>新截图使用的默认颜色和画笔线宽。</span>
                    </div>
                    <div className="quick-ask-setting__controls capture-setting-controls">
                      <label className="capture-setting-field">
                        <span>颜色</span>
                        <input
                          type="color"
                          aria-label="默认标注颜色"
                          value={draft.capture.color}
                          disabled={busy}
                          onChange={(event) =>
                            changeDraft((current) => ({
                              ...current,
                              capture: { ...current.capture, color: event.target.value },
                            }))
                          }
                        />
                      </label>
                      <label className="capture-setting-field">
                        <span>线宽</span>
                        <input
                          type="number"
                          aria-label="默认标注线宽"
                          min={1}
                          max={32}
                          value={draft.capture.lineWidth}
                          disabled={busy}
                          style={{ width: 56 }}
                          onChange={(event) => {
                            const lineWidth = Math.min(32, Math.max(1, Number(event.target.value) || 1));
                            changeDraft((current) => ({
                              ...current,
                              capture: { ...current.capture, lineWidth },
                            }));
                          }}
                        />
                      </label>
                    </div>
                  </div>
                  {accessibility?.diagnostics?.captureShortcutError && (
                    <p className="field-error" role="alert">
                      {accessibility.diagnostics.captureShortcutError}，请更换组合键并保存。
                    </p>
                  )}
                </div>
              )}
            </section>
          )}

          {activeSection === "language" && (
            <section
              className="settings-section settings-section--enter"
              aria-label="语言设置"
            >
              <div className="settings-group" id="language-title">
                <h4 className="settings-group__title">回复偏好</h4>
                <div className="settings-card settings-card--rows">
                  <div className="setting-row">
                    <div>
                      <strong>AI 默认回复语言</strong>
                      <p>仅影响使用 {"{{language}}"} 的动作（如总结、解释），不改变设置界面语言。</p>
                    </div>
                    <StyledSelect
                      className="compact-control"
                      aria-label="AI 默认回复语言"
                      value={draft.locale}
                      onChange={(val) =>
                        changeDraft((current) => ({
                          ...current,
                          locale: val as SupportedLocale,
                        }))
                      }
                      options={[
                        { value: "zh-CN", label: "简体中文" },
                        { value: "en-US", label: "English" },
                      ]}
                    />
                  </div>
                </div>
              </div>
              <div className="settings-group">
                <h4 className="settings-group__title">翻译与查词</h4>
                <div className="settings-card form-grid settings-card--follow">
                  <div className="field">
                    <span className="field__label">翻译主要语言</span>
                    <StyledSelect
                      aria-label="翻译主要语言"
                      value={draft.translate.primaryLanguage}
                      onChange={(val) => {
                        const primaryLanguage = val as TranslationLanguage;
                        changeDraft((current) => {
                          const alternateLanguage =
                            current.translate.alternateLanguage === primaryLanguage
                              ? primaryLanguage === "zh-CN"
                                ? "en-US"
                                : "zh-CN"
                              : current.translate.alternateLanguage;
                          return {
                            ...current,
                            translate: { ...current.translate, primaryLanguage, alternateLanguage },
                          };
                        });
                      }}
                      options={TRANSLATION_LANGUAGES.map((code) => ({
                        value: code,
                        label: TRANSLATION_LANGUAGE_NAMES[code],
                      }))}
                    />
                  </div>
                  <div className="field">
                    <span className="field__label">翻译另一语言</span>
                    <StyledSelect
                      aria-label="翻译另一语言"
                      value={draft.translate.alternateLanguage}
                      onChange={(val) => {
                        const alternateLanguage = val as TranslationLanguage;
                        changeDraft((current) => {
                          const primaryLanguage =
                            current.translate.primaryLanguage === alternateLanguage
                              ? alternateLanguage === "zh-CN"
                                ? "en-US"
                                : "zh-CN"
                              : current.translate.primaryLanguage;
                          return {
                            ...current,
                            translate: { ...current.translate, primaryLanguage, alternateLanguage },
                          };
                        });
                      }}
                      options={TRANSLATION_LANGUAGES.map((code) => ({
                        value: code,
                        label: TRANSLATION_LANGUAGE_NAMES[code],
                      }))}
                    />
                  </div>
                  <div className="field--wide">
                    <SettingSwitch
                      title="英文短文本优先查词"
                      description="内置翻译对 1–5 个英文词调用有道联想与释义；无词条时转 AI。查词无需配置模型。"
                      checked={draft.translate.dictionaryEnabled}
                      onChange={(dictionaryEnabled) =>
                        changeDraft((current) => ({
                          ...current,
                          translate: { ...current.translate, dictionaryEnabled },
                        }))
                      }
                    />
                  </div>
                  <DictionarySettings />
                  <p className="translation-summary field--wide">
                    默认：检测到 {TRANSLATION_LANGUAGE_NAMES[draft.translate.primaryLanguage]} 时译为{" "}
                    {TRANSLATION_LANGUAGE_NAMES[draft.translate.alternateLanguage]}，反之亦然；
                    其他语种默认译为中文，中文默认译为英语。结果框可切换为日语、韩语、俄语、德语、法语等。
                  </p>
                </div>
              </div>
            </section>
          )}
        </div>

        <footer
          className={`settings-footer ${dirty || keysDirty ? "settings-footer--dirty" : ""}`}
          aria-live="polite"
        >
          <span className="settings-footer__status">
            {dirty || keysDirty ? (
              <>
                <span className="settings-footer__dot" aria-hidden="true" />
                有尚未保存的更改
              </>
            ) : (
              <>
                <Check size={15} aria-hidden="true" />
                所有更改均已保存
              </>
            )}
          </span>
          <button
            className="button button--primary settings-save"
            type="button"
            disabled={busy}
            onClick={() => void save()}
            title={
              dirty || keysDirty
                ? "将当前更改写入并立即生效"
                : "当前内容与已保存设置一致，仍可再次保存"
            }
          >
            {operation === "save" ? (
              <LoaderCircle className="settings-spin" size={15} />
            ) : (
              <Check size={15} aria-hidden="true" />
            )}
            保存设置
          </button>
        </footer>
      </div>

      {editor && (
        <CustomActionDialog
          action={editor === "new" ? null : editor}
          providers={draft.providers}
          onCancel={() => setEditor(null)}
          onSave={saveAction}
        />
      )}
      {providerPendingDelete && (
        <ProviderDeleteDialog
          provider={providerPendingDelete}
          onCancel={() => setProviderPendingDelete(null)}
          onConfirm={() => deleteProvider(providerPendingDelete)}
        />
      )}
      {actionPendingDelete && (
        <ActionDeleteDialog
          action={actionPendingDelete}
          onCancel={() => setActionPendingDelete(null)}
          onConfirm={() => deleteAction(actionPendingDelete)}
        />
      )}
      {modelPicker && (
        <ModelPickerDialog
          providerName={modelPicker.providerName}
          previous={modelPicker.previous}
          remoteModels={modelPicker.remoteModels}
          initialCheckedIds={modelPicker.checkedIds}
          onCancel={() => setModelPicker(null)}
          onApply={applyModelPicker}
        />
      )}
    </main>
  );
}

export function ProviderDeleteDialog({
  provider,
  onCancel,
  onConfirm,
}: {
  provider: PublicProviderSettings;
  onCancel: () => void;
  onConfirm: () => void;
}): JSX.Element {
  return (
    <DeleteConfirmationDialog
      title={`删除“${provider.name}”？`}
      description="使用该服务商的动作会变为未配置状态，保存设置时同时清除其密钥。"
      confirmLabel="删除服务商"
      closeLabel="关闭删除确认"
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}

export function ActionDeleteDialog({
  action,
  onCancel,
  onConfirm,
}: {
  action: ActionDefinition;
  onCancel: () => void;
  onConfirm: () => void;
}): JSX.Element {
  return (
    <DeleteConfirmationDialog
      title={`移除动作“${action.name}”？`}
      description="该动作会从工具栏和设置中移除，保存设置后生效。"
      confirmLabel="移除动作"
      closeLabel="关闭动作删除确认"
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}

function DeleteConfirmationDialog({
  title,
  description,
  confirmLabel,
  closeLabel,
  onCancel,
  onConfirm,
}: {
  title: string;
  description: string;
  confirmLabel: string;
  closeLabel: string;
  onCancel: () => void;
  onConfirm: () => void;
}): JSX.Element {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onCancel]);

  return (
    <div
      className="dialog-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <section
        className="dialog-card provider-delete-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={title}
      >
        <header className="dialog-header">
          <div>
            <h2>{title}</h2>
            <p>{description}</p>
          </div>
          <button className="icon-button" type="button" aria-label={closeLabel} onClick={onCancel}>
            <X size={18} aria-hidden="true" />
          </button>
        </header>
        <footer className="dialog-actions provider-delete-dialog__actions">
          <button className="button" type="button" autoFocus onClick={onCancel}>
            取消
          </button>
          <button className="button button--danger" type="button" onClick={onConfirm}>
            {confirmLabel}
          </button>
        </footer>
      </section>
    </div>
  );
}

function SettingSwitch({
  title,
  description,
  checked,
  onChange,
  extra,
}: {
  title: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  extra?: JSX.Element;
}): JSX.Element {
  return (
    <div className="setting-row">
      <div>
        <strong>{title}</strong>
        <p>{description}</p>
      </div>
      <div className="setting-row__end">
        {extra}
        <label className="switch">
          <input
            type="checkbox"
            checked={checked}
            onChange={(event) => onChange(event.target.checked)}
          />
          <span aria-hidden="true" />
          <span className="sr-only">{title}</span>
        </label>
      </div>
    </div>
  );
}

function ActionZone({
  title,
  count,
  actions,
  enabled,
  draggingId,
  onEdit,
  onDelete,
  onToggle,
}: {
  title: string;
  count: string;
  actions: readonly ActionDefinition[];
  enabled: boolean;
  draggingId: string | null;
  onEdit: (action: ActionDefinition) => void;
  onDelete: (action: ActionDefinition) => void;
  onToggle: (action: ActionDefinition) => void;
}): JSX.Element {
  const { isOver, setNodeRef } = useDroppable({ id: actionZoneId(enabled) });

  return (
    <div
      ref={setNodeRef}
      className={`action-zone ${enabled ? "action-zone--enabled" : ""} ${isOver ? "is-drag-over" : ""}`}
      data-action-zone={enabled ? "enabled" : "disabled"}
    >
      <header className="action-zone__header">
        <strong>{title}</strong>
        <span>{count}</span>
      </header>
      <SortableContext
        items={actions.map((action) => action.id)}
        strategy={verticalListSortingStrategy}
      >
        <div className="action-zone__list">
          {actions.map((action) => (
            <SortableActionRow
              key={action.id}
              action={action}
              enabled={enabled}
              dragging={draggingId === action.id}
              onEdit={onEdit}
              onDelete={onDelete}
              onToggle={onToggle}
            />
          ))}
          {actions.length === 0 && <div className="action-zone__empty">拖动动作到这里</div>}
        </div>
      </SortableContext>
    </div>
  );
}

function SortableModelRow({
  model,
  dragging,
  onRemove,
}: {
  model: ProviderModel;
  dragging: boolean;
  onRemove: () => void;
}): JSX.Element {
  const {
    attributes,
    listeners,
    isDragging,
    setActivatorNodeRef,
    setNodeRef,
    transform,
    transition,
  } = useSortable({ id: model.id });
  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };
  const showId = model.name !== model.id;

  return (
    <div
      ref={setNodeRef}
      style={style}
      className={`model-row ${dragging || isDragging ? "is-dragging" : ""}`}
      role="listitem"
      data-model-id={model.id}
    >
      <button
        ref={setActivatorNodeRef}
        className="model-dnd-handle"
        type="button"
        title={`拖动“${model.name}”调整顺序`}
        aria-label={`拖动模型 ${model.name}`}
        {...attributes}
        {...listeners}
      >
        <GripVertical size={14} aria-hidden="true" />
      </button>
      <div className="model-row__body">
        <span className="model-row__name">{model.name}</span>
        {showId && <span className="model-row__id">{model.id}</span>}
      </div>
      <button
        type="button"
        className="model-row__remove"
        aria-label={`移除模型 ${model.name}`}
        onClick={onRemove}
      >
        <X size={12} />
      </button>
    </div>
  );
}

function ModelPickerDialog({
  providerName,
  previous,
  remoteModels,
  initialCheckedIds,
  onCancel,
  onApply,
}: {
  providerName: string;
  previous: readonly ProviderModel[];
  remoteModels: readonly ProviderModel[];
  initialCheckedIds: ReadonlySet<string>;
  onCancel: () => void;
  onApply: (checkedIds: ReadonlySet<string>) => void;
}): JSX.Element {
  const [checkedIds, setCheckedIds] = useState(() => new Set(initialCheckedIds));
  const [query, setQuery] = useState("");

  const listModels = useMemo(() => {
    const byId = new Map<string, ProviderModel>();
    for (const model of remoteModels) {
      if (!byId.has(model.id)) byId.set(model.id, model);
    }
    // Manual / previous-only models stay visible so they can be unchecked.
    for (const model of previous) {
      if (!byId.has(model.id)) byId.set(model.id, model);
    }
    return [...byId.values()];
  }, [previous, remoteModels]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return listModels;
    return listModels.filter(
      (model) =>
        model.id.toLowerCase().includes(needle) || model.name.toLowerCase().includes(needle),
    );
  }, [listModels, query]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onCancel]);

  const toggle = (id: string): void => {
    setCheckedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div
      className="dialog-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <section
        className="dialog-card model-picker-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="model-picker-title"
      >
        <header className="dialog-header">
          <div>
            <h2 id="model-picker-title">选择模型</h2>
            <p>从“{providerName}”获取的模型中勾选要保留的项，应用后请保存设置。</p>
          </div>
          <button
            className="icon-button"
            type="button"
            aria-label="关闭模型选择"
            onClick={onCancel}
          >
            <X size={18} aria-hidden="true" />
          </button>
        </header>
        <div className="model-picker-body">
          <input
            className="control"
            value={query}
            placeholder="搜索模型 ID 或名称"
            aria-label="搜索模型"
            spellCheck={false}
            onChange={(event) => setQuery(event.target.value)}
          />
          <div className="model-picker-list" role="list">
            {filtered.map((model) => {
              const checked = checkedIds.has(model.id);
              const isManual = !remoteModels.some((remote) => remote.id === model.id);
              return (
                <label
                  key={model.id}
                  className={`model-picker-option ${checked ? "is-checked" : ""}`}
                  role="listitem"
                >
                  <input type="checkbox" checked={checked} onChange={() => toggle(model.id)} />
                  <span className="model-picker-option__text">
                    <span className="model-picker-option__name">{model.name}</span>
                    {(model.name !== model.id || isManual) && (
                      <span className="model-picker-option__meta">
                        {model.name !== model.id ? model.id : ""}
                        {isManual ? (model.name !== model.id ? " · 手动" : "手动") : ""}
                      </span>
                    )}
                  </span>
                </label>
              );
            })}
            {filtered.length === 0 && <div className="model-empty">没有匹配的模型</div>}
          </div>
        </div>
        <footer className="dialog-actions model-picker-dialog__actions">
          <button className="button" type="button" onClick={onCancel}>
            取消
          </button>
          <button
            className="button button--primary"
            type="button"
            onClick={() => onApply(checkedIds)}
          >
            应用所选（{checkedIds.size}）
          </button>
        </footer>
      </section>
    </div>
  );
}

function SortableActionRow({
  action,
  enabled,
  dragging,
  onEdit,
  onDelete,
  onToggle,
}: {
  action: ActionDefinition;
  enabled: boolean;
  dragging: boolean;
  onEdit: (action: ActionDefinition) => void;
  onDelete: (action: ActionDefinition) => void;
  onToggle: (action: ActionDefinition) => void;
}): JSX.Element {
  const {
    attributes,
    listeners,
    isDragging,
    setActivatorNodeRef,
    setNodeRef,
    transform,
    transition,
  } = useSortable({ id: action.id, data: { enabled } });
  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <div
      ref={setNodeRef}
      style={style}
      className={`action-dnd-row ${dragging || isDragging ? "is-dragging" : ""}`}
      data-action-id={action.id}
    >
      <button
        ref={setActivatorNodeRef}
        className="action-dnd-handle"
        type="button"
        title={`拖动“${action.name}”调整顺序`}
        aria-label={`拖动${action.name}`}
        {...attributes}
        {...listeners}
      >
        <GripVertical size={16} aria-hidden="true" />
      </button>
      <span className="action-row__icon">
        <ActionIcon name={action.icon} size={17} />
      </span>
      <div className="action-row__copy">
        <strong>{action.name}</strong>
        <span>
          {ACTION_KIND_NAMES[action.kind]}
          {` · ${action.modelId || "使用默认模型"}`}
        </span>
        {isAiActionDefinition(action) && (
          <span className="action-row__prompt" title={action.prompt}>
            提示词：{action.prompt}
          </span>
        )}
      </div>
      <div className="action-row__controls">
        <button
          className="icon-button"
          type="button"
          title="编辑"
          aria-label={`编辑${action.name}`}
          onClick={() => onEdit(action)}
        >
          <Pencil size={15} />
        </button>
        <button
          className="icon-button action-delete"
          type="button"
          title="移除"
          aria-label={`移除${action.name}`}
          onClick={() => onDelete(action)}
        >
          <Trash2 size={15} />
        </button>
        <label className="switch switch--small">
          <input type="checkbox" checked={enabled} onChange={() => onToggle(action)} />
          <span aria-hidden="true" />
          <span className="sr-only">
            {enabled ? "停用" : "启用"}
            {action.name}
          </span>
        </label>
      </div>
    </div>
  );
}
