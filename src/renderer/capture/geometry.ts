import type { CaptureAnnotation, CapturePoint, CaptureRect } from '../../shared/capture';
export type Handle = 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'nw';
export const HANDLES: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
export const clamp = (value: number, min: number, max: number): number => Math.min(max, Math.max(min, value));
export function constrainPoint(point: CapturePoint, bounds: CaptureRect): CapturePoint {
  return { x: clamp(Math.round(point.x), bounds.x, bounds.x + bounds.width), y: clamp(Math.round(point.y), bounds.y, bounds.y + bounds.height) };
}
export function rectangle(a: CapturePoint, b: CapturePoint): CaptureRect {
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.abs(b.x - a.x), height: Math.abs(b.y - a.y) };
}
export function intersect(a: CaptureRect, b: CaptureRect): CaptureRect {
  const x = Math.max(a.x, b.x), y = Math.max(a.y, b.y);
  return { x, y, width: Math.max(0, Math.min(a.x + a.width, b.x + b.width) - x), height: Math.max(0, Math.min(a.y + a.height, b.y + b.height) - y) };
}
export function contains(rect: CaptureRect, point: CapturePoint): boolean {
  return point.x >= rect.x && point.y >= rect.y && point.x <= rect.x + rect.width && point.y <= rect.y + rect.height;
}
export function moveRect(rect: CaptureRect, dx: number, dy: number, bounds: CaptureRect): CaptureRect {
  return { ...rect, x: clamp(Math.round(rect.x + dx), bounds.x, bounds.x + bounds.width - rect.width), y: clamp(Math.round(rect.y + dy), bounds.y, bounds.y + bounds.height - rect.height) };
}
export function resizeRect(rect: CaptureRect, handle: Handle, point: CapturePoint, bounds: CaptureRect): CaptureRect {
  let left = rect.x, top = rect.y, right = rect.x + rect.width, bottom = rect.y + rect.height;
  const p = constrainPoint(point, bounds);
  if (handle.includes('w')) left = Math.min(p.x, right - 1);
  if (handle.includes('e')) right = Math.max(p.x, left + 1);
  if (handle.includes('n')) top = Math.min(p.y, bottom - 1);
  if (handle.includes('s')) bottom = Math.max(p.y, top + 1);
  return intersect({ x: left, y: top, width: right - left, height: bottom - top }, bounds);
}
export function keyRect(rect: CaptureRect, key: string, control: boolean, shift: boolean, bounds: CaptureRect): CaptureRect {
  const dx = key === 'ArrowRight' ? 1 : key === 'ArrowLeft' ? -1 : 0;
  const dy = key === 'ArrowDown' ? 1 : key === 'ArrowUp' ? -1 : 0;
  if (!control && !shift) return moveRect(rect, dx, dy, bounds);
  const delta = shift ? -1 : 1;
  let { x, y, width, height } = rect;
  if (dx < 0) { x -= delta; width += delta; }
  if (dx > 0) width += delta;
  if (dy < 0) { y -= delta; height += delta; }
  if (dy > 0) height += delta;
  if (width < 1 || height < 1) return rect;
  return intersect({ x, y, width, height }, bounds);
}
export function handlePoint(rect: CaptureRect, handle: Handle): CapturePoint {
  return { x: rect.x + (handle.includes('w') ? 0 : handle.includes('e') ? rect.width : rect.width / 2), y: rect.y + (handle.includes('n') ? 0 : handle.includes('s') ? rect.height : rect.height / 2) };
}
export function resizeHandle(rect: CaptureRect, point: CapturePoint, scale: CapturePoint, tolerance = 7): Handle | null {
  const left = Math.abs(point.x - rect.x) * scale.x, right = Math.abs(point.x - rect.x - rect.width) * scale.x;
  const top = Math.abs(point.y - rect.y) * scale.y, bottom = Math.abs(point.y - rect.y - rect.height) * scale.y;
  if (point.x < rect.x - tolerance / scale.x || point.x > rect.x + rect.width + tolerance / scale.x
    || point.y < rect.y - tolerance / scale.y || point.y > rect.y + rect.height + tolerance / scale.y) return null;
  const vertical = Math.min(top, bottom) <= tolerance ? (top <= bottom ? 'n' : 's') : '';
  const horizontal = Math.min(left, right) <= tolerance ? (left <= right ? 'w' : 'e') : '';
  return vertical || horizontal ? (vertical + horizontal) as Handle : null;
}
export function constrainedEnd(start: CapturePoint, end: CapturePoint, tool: string, shift: boolean): CapturePoint {
  if (!shift) return end;
  const dx = end.x - start.x, dy = end.y - start.y;
  if (tool === 'arrow') {
    const angle = Math.round(Math.atan2(dy, dx) / (Math.PI / 4)) * Math.PI / 4;
    const length = Math.hypot(dx, dy);
    return { x: Math.round(start.x + Math.cos(angle) * length), y: Math.round(start.y + Math.sin(angle) * length) };
  }
  const side = Math.max(Math.abs(dx), Math.abs(dy));
  return { x: start.x + Math.sign(dx || 1) * side, y: start.y + Math.sign(dy || 1) * side };
}
export function annotationBounds(item: CaptureAnnotation): CaptureRect {
  const xs = item.points.map((p) => p.x), ys = item.points.map((p) => p.y);
  const x = Math.min(...xs), y = Math.min(...ys);
  if (item.tool === 'text') {
    const size = Math.max(14, item.width * 6), lines = (item.text ?? '').split('\n');
    return { x, y, width: Math.max(1, ...lines.map((line) => Array.from(line).length)) * size, height: lines.length * size * 1.3 };
  }
  return { x: x - item.width, y: y - item.width, width: Math.max(...xs) - x + item.width * 2, height: Math.max(...ys) - y + item.width * 2 };
}
export function hitAnnotation(items: CaptureAnnotation[], point: CapturePoint, tolerance: number): CaptureAnnotation | undefined {
  return [...items].reverse().find((item) => {
    const r = annotationBounds(item);
    return contains({ x: r.x - tolerance, y: r.y - tolerance, width: r.width + 2 * tolerance, height: r.height + 2 * tolerance }, point);
  });
}
export interface AnnotationHistory { past: CaptureAnnotation[][]; present: CaptureAnnotation[]; future: CaptureAnnotation[][] }
export const emptyHistory = (): AnnotationHistory => ({ past: [], present: [], future: [] });
export function commit(history: AnnotationHistory, next: CaptureAnnotation[]): AnnotationHistory {
  return { past: [...history.past, history.present].slice(-100), present: next, future: [] };
}
export function undo(history: AnnotationHistory): AnnotationHistory {
  const previous = history.past.at(-1);
  return previous ? { past: history.past.slice(0, -1), present: previous, future: [history.present, ...history.future] } : history;
}
export function redo(history: AnnotationHistory): AnnotationHistory {
  const next = history.future[0];
  return next ? { past: [...history.past, history.present], present: next, future: history.future.slice(1) } : history;
}
