export interface CaptureRect { x: number; y: number; width: number; height: number }
export interface CapturePoint { x: number; y: number }
export type CaptureTool = 'select' | 'rectangle' | 'ellipse' | 'arrow' | 'pen' | 'text' | 'mosaic';
export type CaptureOutput = 'clipboard' | 'file' | 'pin' | 'ask';
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
  frames: number;
  status: 'appended' | 'tracked' | 'unchanged' | 'unmatched' | 'limit';
}
export interface CaptureDocument { width: number; height: number; version: number }
export interface PinnedImageState {
  id: string;
  width: number;
  height: number;
  zoom: number;
  opacity: number;
  clickThrough: boolean;
  shadowInset: number;
}
export interface CaptureAnnotation {
  id: string;
  tool: Exclude<CaptureTool, 'select'>;
  points: CapturePoint[];
  color: string;
  width: number;
  text?: string;
}
