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
}
export interface CaptureCandidate { rect: CaptureRect; label: string }
export interface PinnedImageState {
  id: string;
  width: number;
  height: number;
  zoom: number;
  opacity: number;
  clickThrough: boolean;
}
export interface CaptureAnnotation {
  id: string;
  tool: Exclude<CaptureTool, 'select'>;
  points: CapturePoint[];
  color: string;
  width: number;
  text?: string;
}
