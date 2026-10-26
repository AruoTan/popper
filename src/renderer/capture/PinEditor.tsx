import { useEffect, useLayoutEffect, useImperativeHandle, useMemo, useRef, useState, type JSX, type Ref, type PointerEvent } from 'react';
import { Check } from 'lucide-react';
import type { CaptureAnnotation, CapturePoint, CaptureTool, PinToolbarAction, PinToolbarState } from '../../shared/capture';
import { annotationBounds, clamp, commit, constrainPoint, constrainedEnd, emptyHistory, hitAnnotation, redo, undo } from './geometry';
import { pngBytes, renderScene } from './drawing';

export interface PinEditorHandle {
  snapshot: () => Promise<Uint8Array>;
  keyDown: (event: KeyboardEvent) => boolean;
  cancelGesture: () => void;
  toolbarAction: (action: PinToolbarAction) => void;
}
type Gesture = { pointer: number; start: CapturePoint; item: CaptureAnnotation; move: boolean; changed: boolean; target: HTMLCanvasElement };
type TextEdit = { item: CaptureAnnotation; text: string };

export function PinEditor({ ref, image, original, initial, toolbar, visible, busy, onToolbarState, onError }: {
  ref: Ref<PinEditorHandle>; image: HTMLImageElement; original: ArrayBuffer; initial: CaptureAnnotation[];
  toolbar: boolean; visible: boolean; busy: boolean; onToolbarState: (state: PinToolbarState) => void;
  onError: (error: unknown) => void;
}): JSX.Element {
  const [history, setHistory] = useState(() => ({ ...emptyHistory(), present: initial }));
  const [tool, setTool] = useState<CaptureTool>('select'), [color, setColor] = useState('#ff4545'), [width, setWidth] = useState(3);
  const [draft, setDraft] = useState<CaptureAnnotation | null>(null), [selected, setSelected] = useState<string | null>(null), [text, setText] = useState<TextEdit | null>(null);
  const canvas = useRef<HTMLCanvasElement>(null), gesture = useRef<Gesture | null>(null), errorHandler = useRef(onError);
  errorHandler.current = onError;
  const stateHandler = useRef(onToolbarState); stateHandler.current = onToolbarState;
  const toolbarState = useMemo<PinToolbarState>(() => ({ tool, color, lineWidth: width, enabled: visible && !busy, busy,
    canUndo: !!history.past.length && !text, canRedo: !!history.future.length && !text, canDelete: !!selected && !text,
  }), [tool, color, width, visible, busy, history.past.length, history.future.length, selected, text]);
  useEffect(() => { if (toolbar) stateHandler.current(toolbarState); }, [toolbar, toolbarState]);
  const bounds = { x: 0, y: 0, width: image.naturalWidth, height: image.naturalHeight };
  const textItems = (): CaptureAnnotation[] => {
    if (!text) return history.present;
    const items = history.present.filter((item) => item.id !== text.item.id);
    return text.text.trim() ? [...items, { ...text.item, text: text.text }] : items;
  };
  const finishText = (): void => { if (text) { setHistory(commit(history, textItems())); setSelected(text.item.id); setText(null); } };
  const cancelGesture = (): void => {
    const current = gesture.current; gesture.current = null; setDraft(null);
    if (current?.target.hasPointerCapture(current.pointer)) current.target.releasePointerCapture(current.pointer);
  };
  useEffect(() => { if (!toolbar || !visible || busy) { cancelGesture(); setSelected(null); } }, [toolbar, visible, busy]);
  useEffect(() => () => { const current = gesture.current; gesture.current = null; if (current?.target.hasPointerCapture(current.pointer)) current.target.releasePointerCapture(current.pointer); }, []);
  const displayed = visible ? textItems().filter((item) => item.id !== draft?.id) : [];
  useLayoutEffect(() => {
    if (!canvas.current) return;
    const items = visible && draft ? [...displayed, draft] : displayed;
    if (!items.length) { canvas.current.width = 0; canvas.current.height = 0; return; }
    try { renderScene(canvas.current, image, bounds.width, bounds.height, items); }
    catch (error) { errorHandler.current(error); }
  }, [image, history, draft, text, visible]);
  const selectedItem = history.present.find((item) => item.id === selected);
  const remove = (): void => { setHistory(commit(history, history.present.filter((item) => item.id !== selected))); setSelected(null); };
  useImperativeHandle(ref, () => ({
    cancelGesture,
    toolbarAction: (action) => {
      if (action.action === 'hide') { finishText(); return; }
      if (!toolbar || !visible || busy) return;
      switch (action.action) {
        case 'tool': finishText(); setTool(action.tool); setSelected(null); break;
        case 'color': setColor(action.color); break;
        case 'width': setWidth(clamp(action.width, 1, 32)); break;
        case 'undo': if (!text) { setHistory(undo); setSelected(null); } break;
        case 'redo': if (!text) { setHistory(redo); setSelected(null); } break;
        case 'delete': if (!text && selected) remove(); break;
      }
    },
    snapshot: async () => {
      cancelGesture();
      const items = visible ? textItems() : [];
      finishText();
      if (!items.length) return new Uint8Array(original);
      const result = document.createElement('canvas');
      try { renderScene(result, image, bounds.width, bounds.height, items); return await pngBytes(result); }
      finally { result.width = 0; result.height = 0; }
    },
    keyDown: (event) => {
      if (!toolbar || !visible || busy || (event.target instanceof Element && event.target.closest('input, textarea'))) return false;
      if (event.ctrlKey && ['z', 'y'].includes(event.key.toLowerCase())) {
        setHistory(event.key.toLowerCase() === 'y' || event.shiftKey ? redo(history) : undo(history)); setSelected(null); return true;
      }
      if (selected && ['Delete', 'Backspace'].includes(event.key)) { remove(); return true; }
      return false;
    },
  }));
  const point = (event: PointerEvent<HTMLCanvasElement>): CapturePoint => {
    const rect = event.currentTarget.getBoundingClientRect();
    return constrainPoint({ x: (event.clientX - rect.left) * bounds.width / rect.width, y: (event.clientY - rect.top) * bounds.height / rect.height }, bounds);
  };
  const start = (event: PointerEvent<HTMLCanvasElement>): void => {
    if (event.button !== 0 || !toolbar || !visible || busy) return;
    if (text) { event.stopPropagation(); finishText(); return; }
    const at = point(event), hit = tool === 'select' ? hitAnnotation(history.present, at, 4 * bounds.width / event.currentTarget.getBoundingClientRect().width) : undefined;
    if (tool === 'select' && !hit) { setSelected(null); return; }
    event.preventDefault(); event.stopPropagation();
    if (tool === 'text') { setText({ item: { id: crypto.randomUUID(), tool, points: [at], color, width }, text: '' }); return; }
    const item: CaptureAnnotation = hit ?? { id: crypto.randomUUID(), tool: tool as Exclude<CaptureTool, 'select'>, points: [at, at], color, width };
    gesture.current = { pointer: event.pointerId, start: at, item, move: !!hit, changed: false, target: event.currentTarget };
    setSelected(item.id); setDraft(item); event.currentTarget.setPointerCapture(event.pointerId);
  };
  const move = (event: PointerEvent<HTMLCanvasElement>): void => {
    const current = gesture.current; if (!current || current.pointer !== event.pointerId) return;
    event.stopPropagation(); const at = point(event);
    current.changed ||= at.x !== current.start.x || at.y !== current.start.y;
    const next = current.move ? { ...current.item, points: current.item.points.map((p) => ({ x: p.x + at.x - current.start.x, y: p.y + at.y - current.start.y })) }
      : { ...current.item, points: current.item.tool === 'pen' ? [...current.item.points, at] : [current.start, constrainPoint(constrainedEnd(current.start, at, current.item.tool, event.shiftKey), bounds)] };
    if (!current.move && current.item.tool === 'pen') current.item = next;
    setDraft(next);
  };
  const end = (event: PointerEvent<HTMLCanvasElement>): void => {
    const current = gesture.current; if (!current || current.pointer !== event.pointerId) return;
    event.stopPropagation();
    if (draft && (current.changed || (!current.move && draft.tool === 'pen'))) setHistory(commit(history, [...history.present.filter((item) => item.id !== draft.id), draft]));
    cancelGesture();
  };
  const selectedBounds = selectedItem && annotationBounds(draft?.id === selectedItem.id ? draft : selectedItem);
  return <>
    <canvas ref={canvas} className="capture-pin-scene" aria-label="贴图标注画布" onPointerDown={start} onPointerMove={move} onPointerUp={end} onPointerCancel={cancelGesture} onLostPointerCapture={cancelGesture}
      onDoubleClick={(event) => {
        if (!toolbar || !visible) return;
        const rect = event.currentTarget.getBoundingClientRect();
        const hit = hitAnnotation(history.present, { x: (event.clientX - rect.left) / rect.width * bounds.width, y: (event.clientY - rect.top) / rect.height * bounds.height }, 4);
        if (hit?.tool === 'text') { event.stopPropagation(); setText({ item: hit, text: hit.text ?? '' }); }
        else if (tool !== 'select') event.stopPropagation();
      }} onWheel={(event) => { if (toolbar && visible && tool !== 'select' && !event.ctrlKey && !busy) { event.preventDefault(); event.stopPropagation(); setWidth(clamp(width + (event.deltaY < 0 ? 1 : -1), 1, 32)); } }} />
    {toolbar && visible && selectedBounds && !text && <span className="capture-pin-object" style={{ left: `${selectedBounds.x / bounds.width * 100}%`, top: `${selectedBounds.y / bounds.height * 100}%`, width: `${selectedBounds.width / bounds.width * 100}%`, height: `${selectedBounds.height / bounds.height * 100}%` }} />}
    {toolbar && visible && text && <div className="capture-pin-text" data-pin-ui onPointerDown={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()}>
      <textarea autoFocus aria-label="标注文字" maxLength={4000} value={text.text} onChange={(event) => setText({ ...text, text: event.target.value })} onKeyDown={(event) => { if (event.ctrlKey && event.key === 'Enter') { event.preventDefault(); finishText(); } }} />
      <button onClick={finishText}><Check size={14} />完成文字</button>
    </div>}
  </>;
}
