import { useEffect, useLayoutEffect, useImperativeHandle, useRef, useState, type JSX, type Ref, type PointerEvent } from 'react';
import { Check, Copy, Download, Redo2, Trash2, Undo2, X } from 'lucide-react';
import type { CaptureAnnotation, CapturePoint, CaptureTool } from '../../shared/capture';
import { ANNOTATION_TOOLS } from './annotationTools';
import { annotationBounds, clamp, commit, constrainPoint, constrainedEnd, emptyHistory, hitAnnotation, redo, undo } from './geometry';
import { pngBytes, renderScene } from './drawing';

export interface PinEditorHandle {
  snapshot: () => Promise<Uint8Array>;
  keyDown: (event: KeyboardEvent) => boolean;
  cancelGesture: () => void;
}
type Gesture = { pointer: number; start: CapturePoint; item: CaptureAnnotation; move: boolean; changed: boolean; target: HTMLCanvasElement };
type TextEdit = { item: CaptureAnnotation; text: string };

export function PinEditor({ ref, image, original, initial, toolbar, visible, busy, onOutput, onHideToolbar, onError }: {
  ref: Ref<PinEditorHandle>; image: HTMLImageElement; original: ArrayBuffer; initial: CaptureAnnotation[];
  toolbar: boolean; visible: boolean; busy: boolean; onOutput: (output: 'copy' | 'save') => void;
  onHideToolbar: () => void; onError: (error: unknown) => void;
}): JSX.Element {
  const [history, setHistory] = useState(() => ({ ...emptyHistory(), present: initial }));
  const [tool, setTool] = useState<CaptureTool>('select'), [color, setColor] = useState('#ff4545'), [width, setWidth] = useState(3);
  const [draft, setDraft] = useState<CaptureAnnotation | null>(null), [selected, setSelected] = useState<string | null>(null), [text, setText] = useState<TextEdit | null>(null);
  const canvas = useRef<HTMLCanvasElement>(null), gesture = useRef<Gesture | null>(null), errorHandler = useRef(onError);
  errorHandler.current = onError;
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
    {toolbar && <div className="capture-pin-toolbar" role="toolbar" aria-label="贴图工具条" data-pin-ui onPointerDown={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()} onWheel={(event) => event.stopPropagation()}>
      {ANNOTATION_TOOLS.map(([value, Icon, title]) => <button key={value} title={title} aria-label={title} aria-pressed={tool === value} disabled={busy || !visible} onClick={() => { finishText(); setTool(value); setSelected(null); }}><Icon size={16} /></button>)}
      <input type="color" aria-label="标注颜色" value={color} disabled={busy || !visible} onChange={(event) => setColor(event.target.value)} />
      <input type="number" aria-label="线宽" min={1} max={32} value={width} disabled={busy || !visible} onChange={(event) => setWidth(clamp(Number(event.target.value) || 1, 1, 32))} />
      <button aria-label="撤销" title="撤销" disabled={busy || !visible || !history.past.length || !!text} onClick={() => { setHistory(undo(history)); setSelected(null); }}><Undo2 size={16} /></button>
      <button aria-label="重做" title="重做" disabled={busy || !visible || !history.future.length || !!text} onClick={() => { setHistory(redo(history)); setSelected(null); }}><Redo2 size={16} /></button>
      <button aria-label="删除标注" title="删除标注" disabled={busy || !visible || !selected || !!text} onClick={remove}><Trash2 size={16} /></button>
      <button aria-label="复制图像" title="复制图像" disabled={busy} onClick={() => onOutput('copy')}><Copy size={16} /></button>
      <button aria-label="另存图像" title="另存图像" disabled={busy} onClick={() => onOutput('save')}><Download size={16} /></button>
      <button aria-label="隐藏工具条" title="隐藏工具条" disabled={busy} onClick={() => { finishText(); onHideToolbar(); }}><X size={16} /></button>
    </div>}
    {toolbar && visible && text && <div className="capture-pin-text" data-pin-ui onPointerDown={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()}>
      <textarea autoFocus aria-label="标注文字" maxLength={4000} value={text.text} onChange={(event) => setText({ ...text, text: event.target.value })} onKeyDown={(event) => { if (event.ctrlKey && event.key === 'Enter') { event.preventDefault(); finishText(); } }} />
      <button onClick={finishText}><Check size={14} />完成文字</button>
    </div>}
  </>;
}
