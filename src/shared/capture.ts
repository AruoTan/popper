export interface CaptureRect { x: number; y: number; width: number; height: number }
export interface CapturePoint { x: number; y: number }
export type CaptureTool = 'select' | 'rectangle' | 'ellipse' | 'arrow' | 'pen' | 'text' | 'mosaic';
export type CaptureOutput = 'clipboard' | 'file' | 'pin' | 'ask';
export type CaptureColorFormat = 'hex' | 'rgb';
export interface CaptureSession {
  id: string;
  screen: CaptureRect;
  scale: number;
  color: string;
  lineWidth: number;
  targets: CaptureTarget[];
  pointer: CapturePoint | null;
}
export interface CaptureTarget { rect: CaptureRect; label: string; controls: CaptureCandidate[] }
export interface CaptureCandidate { rect: CaptureRect; label: string }
export interface CaptureHitTest { candidates: CaptureCandidate[]; pending: boolean }
export interface ScrollCaptureProgress {
  width: number;
  height: number;
  revision: number;
  viewport: CaptureRect;
  status: 'appended' | 'tracked' | 'unchanged' | 'unmatched' | 'limit';
}
export interface ScrollCaptureSession {
  progress: ScrollCaptureProgress;
  layout: { selection: CaptureRect; panel: CaptureRect; toolbar: CaptureRect };
}
export interface CaptureDocument { width: number; height: number; version: number }
export interface PinnedImageState {
  id: string;
  width: number;
  height: number;
  zoom: number;
  opacity: number;
  shadowInset: number;
  toolbarVisible: boolean;
  annotationsVisible: boolean;
  alwaysOnTop: boolean;
}
export interface CaptureAnnotation {
  id: string;
  tool: Exclude<CaptureTool, 'select'>;
  points: CapturePoint[];
  color: string;
  width: number;
  text?: string;
}
export interface PinEditorState {
  history: { past: CaptureAnnotation[][]; present: CaptureAnnotation[]; future: CaptureAnnotation[][] };
  tool: CaptureTool;
  color: string;
  lineWidth: number;
  selected: string | null;
  text: { item: CaptureAnnotation; text: string } | null;
}
export interface PinToolbarState {
  tool: CaptureTool;
  color: string;
  lineWidth: number;
  enabled: boolean;
  busy: boolean;
  canUndo: boolean;
  canRedo: boolean;
  canDelete: boolean;
}
export type PinToolbarAction =
  | { action: 'tool'; tool: CaptureTool }
  | { action: 'color'; color: string }
  | { action: 'width'; width: number }
  | { action: 'undo' | 'redo' | 'delete' | 'copy' | 'save' | 'hide' }
  | { action: 'close'; destroy: boolean };
