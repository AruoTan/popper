import type {
  DictionarySnapshot,
  DictionarySuggestion,
  StudyBook,
  TranslationSubmission,
} from "./dictionary";
import type {
  ActionStreamEvent,
  Point,
  ProviderCreateInput,
  ProviderModel,
  ProviderUpdateInput,
  PublicSettings,
  ResultReadyAck,
  ResultRendererMarker,
  ResultSessionSnapshot,
  SelectionPayload,
  SettingsUpdate,
  ToolbarDismissedEvent,
  ToolbarPointerEvent,
  ToolbarSize,
  TranslationLanguage,
} from "./schemas";

export type Unsubscribe = () => void;

export interface RuntimeDiagnostics {
  selectionMonitorError?: string;
}

export interface AccessibilityStatus {
  platform: "darwin" | "windows" | "unsupported";
  trusted: boolean;
  canRequest: boolean;
  /** Whether selection capture is usable, including runtime hook startup. */
  available: boolean;
  /** Replayable, non-sensitive failures retained by the native runtime. */
  diagnostics: RuntimeDiagnostics;
}

export type ConnectionTestResult =
  | { ok: true; models: ProviderModel[] }
  | { ok: false; message: string; status?: number };

export type ProviderModelsSyncResult =
  | { ok: true; models: ProviderModel[]; settings?: PublicSettings }
  | { ok: false; message: string; status?: number };

export type RunActionResult =
  | { accepted: true; sessionId?: string; requestId?: string }
  | { accepted: false; message: string };

export interface ActionRetryOptions {
  targetLanguage?: TranslationLanguage;
  providerId?: string;
  modelId?: string;
}

export type SettingsFocusSection = "actions" | "providers";

export interface OpenSettingsOptions {
  focus?: SettingsFocusSection | string;
  notice?: string;
}

export interface SettingsGuidance {
  focus?: SettingsFocusSection | string;
  notice?: string;
}

/** Temporary Windows runtime report; remove after desktop offset validation. */
export interface SelectionClickPointHealth {
  point: Point;
  inSelection: boolean;
  geometryValid: boolean | null;
  distance: number | null;
}

export interface SelectionClickValidation {
  down: SelectionClickPointHealth;
  up: SelectionClickPointHealth;
  selectionRectangles: { x: number; y: number; width: number; height: number }[];
}

export interface SelectionDetectionHealth {
  status: "NORMAL" | "OFFSET_DETECTED" | "SUSPICIOUS" | "UIA_UNAVAILABLE";
  applicable: boolean;
  reason: string;
  selectionDirection: string;
  startEndpointValid: boolean | null;
  endEndpointValid: boolean | null;
  startGeometryValid: boolean | null;
  endGeometryValid: boolean | null;
  startDistance: number | null;
  endDistance: number | null;
  confidence: number;
  stable: boolean;
  clickValidation?: SelectionClickValidation | null;
}

export interface SelectionDetectionDebug {
  version: string;
  captureId: number;
  selectionTimestampMs: number;
  dpiReady: boolean;
  gesture: { kind: string; down: { x: number; y: number }; up: { x: number; y: number } } | null;
  fallbackAttempted: boolean;
  copyInjected?: boolean;
  provider: string | null;
  method: string | null;
  outcome: string;
  selectionText: string | null;
  initialHealth?: SelectionDetectionHealth;
  health: SelectionDetectionHealth;
  durationMs: number;
  logPath?: string | null;
  steps: { stage: string; elapsedMs: number; details: unknown }[];
  inputEvents?: unknown[];
  stepsTruncated?: boolean;
}

export interface WindowPopperApi {
  /** Runtime only; resets to false on every app launch. */
  getSelectionDetectionDebugEnabled?(): Promise<boolean>;
  setSelectionDetectionDebugEnabled?(enabled: boolean): Promise<boolean>;
  onSelectionDetectionDebugChanged?(listener: (enabled: boolean) => void): Unsubscribe;
  /** Result window only, scoped to its original selection timestamp. */
  getSelectionDetectionDebug?(sessionId: string): Promise<SelectionDetectionDebug | null>;
  submitTranslation?(sessionId: string, text: string): Promise<TranslationSubmission>;
  translationInputSuggestions?(
    sessionId: string,
    requestId: string,
    version: number,
    text: string,
  ): Promise<DictionarySuggestion[]>;
  getDictionaryState?(sessionId: string): Promise<DictionarySnapshot | null>;
  queryDictionary?(sessionId: string, query: string): Promise<string>;
  suggestDictionary?(
    sessionId: string,
    query: string,
    queryGeneration: number,
  ): Promise<DictionarySuggestion[]>;
  cancelDictionaryInput?(sessionId: string, queryGeneration: number): Promise<void>;
  dictionaryAudio?(sessionId: string, accent: 1 | 2): Promise<string>;
  getEudicBooks?(sessionId: string): Promise<StudyBook[]>;
  addEudicWord?(sessionId: string, queryGeneration: number, categoryId: string): Promise<void>;
  eudicConfigured?(): Promise<boolean>;
  setEudicAuthorization?(authorization: string): Promise<void>;
  onDictionaryChanged?(listener: (snapshot: DictionarySnapshot) => void): Unsubscribe;
  getSettings(): Promise<PublicSettings>;
  updateSettings(update: SettingsUpdate): Promise<PublicSettings>;
  resetResultSize?(): Promise<PublicSettings>;
  createProvider?(input: ProviderCreateInput): Promise<PublicSettings>;
  updateProvider?(providerId: string, update: ProviderUpdateInput): Promise<PublicSettings>;
  deleteProvider?(providerId: string): Promise<PublicSettings>;
  setProviderApiKey?(providerId: string, apiKey: string): Promise<PublicSettings>;
  clearProviderApiKey?(providerId: string): Promise<PublicSettings>;
  /** Settings window only: load a saved provider key for masked display. */
  getProviderApiKey?(providerId: string): Promise<string | null>;
  testProviderConnection?(providerId: string): Promise<ConnectionTestResult>;
  /** List remote models without writing settings (selective pick). */
  listProviderModels?(providerId: string): Promise<ConnectionTestResult>;
  syncProviderModels?(providerId: string): Promise<ProviderModelsSyncResult>;
  /** @deprecated Use setProviderApiKey. */
  setApiKey?(apiKey: string): Promise<PublicSettings>;
  /** @deprecated Use clearProviderApiKey. */
  clearApiKey?(): Promise<PublicSettings>;
  /** @deprecated Use testProviderConnection. */
  testConnection?(): Promise<ConnectionTestResult>;
  getAccessibilityStatus(): Promise<AccessibilityStatus>;
  requestAccessibility(): Promise<AccessibilityStatus>;
  getCurrentSelection?(): Promise<SelectionPayload | null>;
  /** Windows-only hidden prepare/visible commit handshake for the toolbar. */
  presentToolbar?(selectionId: string, size: ToolbarSize): Promise<boolean>;
  /** Internal Windows-only recovery for an unresponsive singleton toolbar. */
  recoverToolbar?(selectionId: string): Promise<boolean>;
  runAction(
    actionId: string,
    cursor?: Point,
    selectionId?: string,
    initialQuestion?: string,
  ): Promise<RunActionResult>;
  /** Temporarily lets the selection toolbar accept keyboard input. */
  setToolbarInputMode?(active: boolean, selectionId?: string): Promise<boolean>;
  /** Activates the toolbar after its inline input layout has been committed. */
  focusToolbarInput?(selectionId?: string): Promise<boolean>;
  hideToolbar(selectionId?: string): Promise<void>;
  reportToolbarSize(size: ToolbarSize, selectionId?: string): Promise<void>;
  beginResultReady(sessionId: string): Promise<ResultSessionSnapshot>;
  ackResultReady(ack: ResultReadyAck): Promise<boolean>;
  recordResultRendererMarker(
    sessionId: string,
    requestId: string,
    marker: ResultRendererMarker,
  ): Promise<void>;
  /** Internal result-window reveal handshake. */
  prepareResultReveal?(sessionId: string): Promise<void>;
  /** Internal result-window reveal handshake. */
  commitResultReveal?(sessionId: string): Promise<void>;
  /** Reports a renderer hydration failure before the result becomes visible. */
  failResultReveal?(sessionId: string, message: string): Promise<void>;
  setResultPinned?(sessionId: string, pinned: boolean): Promise<boolean | void>;
  setResultPointerInside?(sessionId: string, inside: boolean): Promise<void>;
  showResultSelection?(
    sessionId: string,
    text: string,
    cursor: Point,
    forceCapture?: boolean,
  ): Promise<void>;
  /** Result window only: dismiss toolbar opened from an in-result selection. */
  hideResultSelection?(sessionId: string): Promise<void>;
  cancelAction(sessionId?: string): Promise<void>;
  retryAction(sessionId?: string, options?: ActionRetryOptions): Promise<RunActionResult>;
  continueAction?(sessionId: string, question: string): Promise<RunActionResult>;
  copyText(text: string): Promise<void>;
  openExternal(url: string): Promise<void>;
  openSettings?(options?: OpenSettingsOptions): Promise<void>;
  /** Settings window only: consume one-shot open guidance (focus + notice). */
  takeSettingsGuidance?(): Promise<SettingsGuidance | null>;
  hideResult?(sessionId?: string): Promise<void>;
  closeResult(sessionId?: string): Promise<void>;
  onSelection(listener: (selection: SelectionPayload) => void): Unsubscribe;
  onActionEvent(listener: (event: ActionStreamEvent) => void): Unsubscribe;
  onSettingsChanged(listener: (settings: PublicSettings) => void): Unsubscribe;
  /** Settings window only: reset transient presentation on each explicit open. */
  onSettingsOpened?(listener: () => void): Unsubscribe;
  onSettingsGuidance?(listener: (guidance: SettingsGuidance) => void): Unsubscribe;
  /** Result window only: native right-button hold requests the current DOM selection. */
  onResultSelectionHold?(listener: () => void): Unsubscribe;
  /**
   * Native pointer movement for the non-activating selection toolbar.
   * Optional so a renderer can still hot-reload against an older backend.
   */
  onToolbarPointer?(listener: (event: ToolbarPointerEvent) => void): Unsubscribe;
  /**
   * Native force-dismiss of the selection toolbar (outside click / Escape).
   * Optional so a renderer can still hot-reload against an older backend.
   */
  onToolbarDismissed?(listener: (payload: ToolbarDismissedEvent) => void): Unsubscribe;
}

declare global {
  interface Window {
    _popper_: WindowPopperApi;
    /** @deprecated Use _popper_. */
    selectionBar: WindowPopperApi;
  }
}
