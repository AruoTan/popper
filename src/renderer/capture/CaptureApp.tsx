import { useEffect, useRef, useState, type JSX, type PointerEvent as ReactPointerEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowUpRight, Check, Circle, Copy, Download, MessageSquare, MousePointer2, Pencil, Pin, Redo2, Square, Type, Undo2, X, Grid3X3, Trash2 } from 'lucide-react';
import type { CaptureAnnotation, CaptureCandidate, CaptureOutput, CapturePoint, CaptureRect, CaptureSession, CaptureTool } from '../../shared/capture';
import { getErrorMessage } from '../lib/errors';
import { annotationBounds, clamp, commit, constrainPoint, constrainedEnd, contains, emptyHistory, HANDLES, handlePoint, hitAnnotation, keyRect, moveRect, rectangle, redo, resizeRect, undo, type Handle } from './geometry';
import { decodePng, pngBytes, renderScene } from './drawing';

type Gesture = { mode: 'new' | 'move' | 'resize' | 'draw' | 'object'; start: CapturePoint; rect: CaptureRect | null; handle?: Handle; item?: CaptureAnnotation; moved: boolean };
type TextEdit = { id: string; point: CapturePoint; text: string; color: string; width: number };
const TOOLS = [ ['select', MousePointer2, '选择与调整'], ['rectangle', Square, '矩形'], ['ellipse', Circle, '椭圆'], ['arrow', ArrowUpRight, '箭头'], ['pen', Pencil, '画笔'], ['text', Type, '文字'], ['mosaic', Grid3X3, '马赛克'] ] as const;

function Magnifier({ image, point, screen }: { image: HTMLImageElement; point: CapturePoint; screen: CaptureRect }): JSX.Element {
  const canvas = useRef<HTMLCanvasElement>(null); const [color, setColor] = useState('');
  useEffect(() => {
    const ctx = canvas.current?.getContext('2d', { willReadFrequently: true }); if (!ctx) return;
    const x = clamp(point.x, 0, screen.width - 1), y = clamp(point.y, 0, screen.height - 1);
    ctx.fillStyle = '#15171b'; ctx.fillRect(0, 0, 143, 143); ctx.imageSmoothingEnabled = false;
    const left = Math.max(0, x - 5), top = Math.max(0, y - 5), right = Math.min(screen.width, x + 6), bottom = Math.min(screen.height, y + 6);
    ctx.drawImage(image, left, top, right - left, bottom - top, (left - x + 5) * 13, (top - y + 5) * 13, (right - left) * 13, (bottom - top) * 13);
    const pixel = ctx.getImageData(71, 71, 1, 1).data;
    setColor('#' + Array.from(pixel.slice(0, 3)).map((n) => n.toString(16).padStart(2, '0')).join('').toUpperCase());
    ctx.strokeStyle = 'rgba(0,0,0,.2)'; ctx.lineWidth = 1;
    for (let i = 0; i <= 11; i++) { ctx.beginPath(); ctx.moveTo(i * 13, 0); ctx.lineTo(i * 13, 143); ctx.moveTo(0, i * 13); ctx.lineTo(143, i * 13); ctx.stroke(); }
    ctx.strokeStyle = '#fff'; ctx.strokeRect(65.5, 65.5, 12, 12);
  }, [image, point, screen]);
  const px = point.x / screen.width * window.innerWidth, py = point.y / screen.height * window.innerHeight;
  return <aside className="capture-magnifier" style={{ left: clamp(px + 24, 8, window.innerWidth - 168), top: clamp(py + 28, 8, window.innerHeight - 205) }}>
    <canvas ref={canvas} width={143} height={143} /><span>{screen.x + point.x}, {screen.y + point.y}</span><span>{color}</span>
  </aside>;
}

export function CaptureApp(): JSX.Element {
  const [session, setSession] = useState<CaptureSession | null>(null), [image, setImage] = useState<HTMLImageElement | null>(null);
  const [selection, setSelection] = useState<CaptureRect | null>(null), [candidates, setCandidates] = useState<CaptureCandidate[]>([]), [candidateIndex, setCandidateIndex] = useState(0);
  const [point, setPoint] = useState<CapturePoint>({ x: 0, y: 0 }), [tool, setTool] = useState<CaptureTool>('select');
  const [history, setHistory] = useState(emptyHistory), [draft, setDraft] = useState<CaptureAnnotation | null>(null), [selectedId, setSelectedId] = useState<string | null>(null);
  const [textEdit, setTextEdit] = useState<TextEdit | null>(null), [color, setColor] = useState('#ff4545'), [lineWidth, setLineWidth] = useState(3);
  const [error, setError] = useState(''), [busy, setBusy] = useState(false), [alt, setAlt] = useState(false), [viewport, setViewport] = useState({ width: window.innerWidth, height: window.innerHeight });
  const keyboard = useRef<(event: KeyboardEvent) => void>(() => {});
  const canvas = useRef<HTMLCanvasElement>(null), root = useRef<HTMLElement>(null), gesture = useRef<Gesture | null>(null), busyRef = useRef(false), version = useRef(0), generation = useRef(0);
  const queryPending = useRef(false), queryTimer = useRef<number | undefined>(undefined), lastPoint = useRef(point), mounted = useRef(true), selectedRef = useRef(selection);
  selectedRef.current = selection;
  const bounds: CaptureRect = { x: 0, y: 0, width: session?.screen.width ?? 1, height: session?.screen.height ?? 1 };
  const sx = viewport.width / bounds.width, sy = viewport.height / bounds.height;
  const candidate = candidates[candidateIndex];
  const displayed = selection ?? candidate?.rect ?? null;
  const selectedItem = history.present.find((item) => item.id === selectedId);
  const displayedItems = history.present.filter((item) => item.id !== textEdit?.id && item.id !== (gesture.current?.mode === 'object' ? draft?.id : undefined));

  const cancel = (): void => {
    if (busyRef.current) return;
    busyRef.current = true; setBusy(true);
    void invoke('capture_cancel').catch((e: unknown) => { busyRef.current = false; setBusy(false); setError(getErrorMessage(e, '无法结束截图')); });
  };
  useEffect(() => {
    mounted.current = true; let alive = true;
    void (async () => {
      try {
        const [meta, bytes] = await Promise.all([invoke<CaptureSession>('capture_session'), invoke<ArrayBuffer>('capture_image')]);
        const decoded = await decodePng(bytes); if (!alive) return;
        setSession(meta); setImage(decoded); setColor(meta.color); setLineWidth(meta.lineWidth);
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
        if (alive) { await invoke('capture_show'); root.current?.focus(); }
      } catch (e) { if (alive) {
        const message = getErrorMessage(e, '无法初始化屏幕图像'); setError(message);
        void invoke('capture_fail', { message }).catch(() => {});
      } }
    })();
    const resize = (): void => setViewport({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener('resize', resize);
    const keyDown = (event: KeyboardEvent): void => keyboard.current(event);
    const keyUp = (event: KeyboardEvent): void => { if (event.key === 'Alt') setAlt(false); };
    const focus = (): void => root.current?.focus();
    window.addEventListener('keydown', keyDown);
    window.addEventListener('keyup', keyUp);
    window.addEventListener('focus', focus);
    const blur = (): void => { if (!busyRef.current && alive) void invoke('capture_cancel').catch(() => {}); };
    window.addEventListener('blur', blur);
    return () => { alive = false; mounted.current = false; window.removeEventListener('resize', resize); window.removeEventListener('blur', blur); window.removeEventListener('keydown', keyDown); window.removeEventListener('keyup', keyUp); window.removeEventListener('focus', focus); clearTimeout(queryTimer.current); generation.current++; };
  }, []);
  useEffect(() => {
    if (!canvas.current || !image) return;
    try { renderScene(canvas.current, image, bounds.width, bounds.height, draft ? [...displayedItems, draft] : displayedItems); }
    catch (e) { setError(getErrorMessage(e, '无法绘制标注')); }
  }, [image, history, draft, textEdit, bounds.width, bounds.height]);

  useEffect(() => {
    if (!session || busy) return;
    const timer = window.setTimeout(() => { void invoke('capture_preferences', { color, lineWidth }).catch(() => {}); }, 250);
    return () => clearTimeout(timer);
  }, [session, color, lineWidth, busy]);

  const query = (p: CapturePoint): void => {
    lastPoint.current = p; generation.current++;
    if (queryPending.current || queryTimer.current !== undefined) return;
    queryTimer.current = window.setTimeout(() => {
      queryTimer.current = undefined; if (selectedRef.current || gesture.current || !mounted.current) return;
      queryPending.current = true; const sequence = generation.current; const at = lastPoint.current;
      void invoke<CaptureCandidate[]>('capture_candidates', { x: at.x, y: at.y, generation: sequence }).then((items) => {
        if (mounted.current && generation.current === sequence && !selectedRef.current && !gesture.current) { setCandidates(items); setCandidateIndex(0); }
      }).catch(() => {}).finally(() => { queryPending.current = false; if (mounted.current && generation.current !== sequence && !selectedRef.current) query(lastPoint.current); });
    }, 60);
  };
  const eventPoint = (event: { clientX: number; clientY: number }): CapturePoint => constrainPoint({ x: event.clientX / sx, y: event.clientY / sy }, bounds);
  const finishText = (): void => {
    if (!textEdit) return;
    const next = history.present.filter((item) => item.id !== textEdit.id);
    if (textEdit.text.trim()) next.push({ id: textEdit.id, tool: 'text', points: [textEdit.point], text: textEdit.text, color: textEdit.color, width: textEdit.width });
    setHistory(commit(history, next)); setSelectedId(textEdit.id); setTextEdit(null);
  };
  const output = async (destination: CaptureOutput): Promise<void> => {
    if (busyRef.current || !selection || !image || textEdit) return;
    busyRef.current = true; setBusy(true); setError(''); gesture.current = null; setDraft(null);
    try {
      await invoke('capture_preferences', { color, lineWidth }).catch(() => {});
      const revision = ++version.current;
      const original = await invoke<ArrayBuffer>('capture_crop', { rect: selection, version: revision });
      let bytes: Uint8Array;
      if (!history.present.length) bytes = new Uint8Array(original);
      else {
        const scene = document.createElement('canvas');
        renderScene(scene, image, bounds.width, bounds.height, history.present);
        const result = document.createElement('canvas'); result.width = selection.width; result.height = selection.height;
        const context = result.getContext('2d'); if (!context) throw new Error('无法创建导出画布');
        context.drawImage(scene, selection.x, selection.y, selection.width, selection.height, 0, 0, selection.width, selection.height);
        bytes = await pngBytes(result); scene.width = 0; scene.height = 0;
      }
      const finished = await invoke<boolean>('capture_export', bytes, { headers: { 'x-capture-version': String(revision), 'x-capture-output': destination } });
      if (!finished && mounted.current) { busyRef.current = false; setBusy(false); root.current?.focus(); }
    } catch (e) { if (mounted.current) { setError(getErrorMessage(e, '无法输出截图')); busyRef.current = false; setBusy(false); root.current?.focus(); } }
  };
  const start = (event: ReactPointerEvent<HTMLElement>): void => {
    if (event.button !== 0 || busyRef.current || !session || !image || (event.target as HTMLElement).closest('[data-capture-ui]')) return;
    if (textEdit) { finishText(); return; }
    event.preventDefault(); root.current?.focus();
    const p = eventPoint(event); setPoint(p); setError('');
    root.current?.setPointerCapture(event.pointerId);
    const handle = selection && HANDLES.find((h) => { const hp = handlePoint(selection, h); return Math.abs(hp.x - p.x) * sx <= 7 && Math.abs(hp.y - p.y) * sy <= 7; });
    if (handle && selection) gesture.current = { mode: 'resize', start: p, rect: selection, handle, moved: false };
    else if (tool !== 'select' && selection && contains(selection, p)) {
      if (tool === 'text') {
        if (root.current?.hasPointerCapture(event.pointerId)) root.current.releasePointerCapture(event.pointerId);
        setTextEdit({ id: crypto.randomUUID(), point: p, text: '', color, width: lineWidth }); return;
      }
      const item: CaptureAnnotation = { id: crypto.randomUUID(), tool, points: [p, p], color, width: lineWidth };
      gesture.current = { mode: 'draw', start: p, rect: selection, item, moved: false }; setDraft(item);
    } else if (selection && contains(selection, p)) {
      const item = hitAnnotation(history.present, p, 4 / sx);
      if (item) { gesture.current = { mode: 'object', start: p, rect: selection, item, moved: false }; setSelectedId(item.id); }
      else { gesture.current = { mode: 'move', start: p, rect: selection, moved: false }; setSelectedId(null); }
    } else { gesture.current = { mode: 'new', start: p, rect: null, moved: false }; setSelectedId(null); }
  };
  const move = (event: ReactPointerEvent<HTMLElement>): void => {
    if (!session || busyRef.current) return;
    const p = eventPoint(event); setPoint(p);
    const current = gesture.current;
    if (!current) { if (!selection) query(p); return; }
    const dx = p.x - current.start.x, dy = p.y - current.start.y;
    if (Math.hypot(dx * sx, dy * sy) > 4) current.moved = true;
    if (current.mode === 'new' && current.moved) { setSelection(rectangle(current.start, p)); setCandidates([]); }
    if (current.mode === 'move' && current.rect) setSelection(moveRect(current.rect, dx, dy, bounds));
    if (current.mode === 'resize' && current.rect && current.handle) setSelection(resizeRect(current.rect, current.handle, p, bounds));
    if (current.mode === 'draw' && current.item) {
      const end = constrainPoint(constrainedEnd(current.start, p, current.item.tool, event.shiftKey), bounds);
      if (current.item.tool === 'pen') current.item = { ...current.item, points: [...current.item.points, p] };
      else current.item = { ...current.item, points: [current.start, end] };
      setDraft(current.item);
    }
    if (current.mode === 'object' && current.item && current.moved) setDraft({ ...current.item, points: current.item.points.map((point) => ({ x: point.x + dx, y: point.y + dy })) });
  };
  const end = (event: ReactPointerEvent<HTMLElement>): void => {
    const current = gesture.current; if (!current) return;
    if (root.current?.hasPointerCapture(event.pointerId)) root.current.releasePointerCapture(event.pointerId);
    gesture.current = null;
    if (current.mode === 'new' && !current.moved) {
      const p = eventPoint(event); const sequence = ++generation.current;
      if (candidate && contains(candidate.rect, p)) { setSelection(candidate.rect); setDraft(null); return; }
      void invoke<CaptureCandidate[]>('capture_candidates', { x: p.x, y: p.y, generation: sequence }).then((items) => {
        if (mounted.current && generation.current === sequence && !gesture.current) setSelection(items[0]?.rect ?? { x: Math.min(p.x, bounds.width - 1), y: Math.min(p.y, bounds.height - 1), width: 1, height: 1 });
      }).catch(() => { if (mounted.current && generation.current === sequence) setSelection({ x: Math.min(p.x, bounds.width - 1), y: Math.min(p.y, bounds.height - 1), width: 1, height: 1 }); });
    }
    if (current.mode === 'draw' && draft && (current.moved || draft.tool === 'pen')) { setHistory(commit(history, [...history.present, draft])); setSelectedId(draft.id); }
    if (current.mode === 'object' && current.moved && draft) setHistory(commit(history, history.present.map((item) => item.id === draft.id ? draft : item)));
    setDraft(null);
    if (selection && selection.width > 0 && selection.height > 0) void invoke('capture_select', { rect: selection, version: ++version.current }).catch(() => {});
  };
  keyboard.current = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') { event.preventDefault(); cancel(); return; }
      if (busyRef.current) return;
      if ((event.target as HTMLElement).closest('input, textarea')) return;
      if (event.key === 'Alt') { event.preventDefault(); setAlt(true); }
      if (event.key === 'Tab' && !selection) { event.preventDefault(); setCandidateIndex((index) => clamp(index + (event.shiftKey ? -1 : 1), 0, Math.max(0, candidates.length - 1))); }
      if (event.key.startsWith('Arrow') && selection && !textEdit) { event.preventDefault(); setSelection(keyRect(selection, event.key, event.ctrlKey, event.shiftKey, bounds)); }
      if ((event.key === 'Delete' || event.key === 'Backspace') && selectedId) { event.preventDefault(); setHistory(commit(history, history.present.filter((item) => item.id !== selectedId))); setSelectedId(null); }
      if (event.ctrlKey && event.key.toLowerCase() === 'z') { event.preventDefault(); setHistory(event.shiftKey ? redo(history) : undo(history)); setSelectedId(null); }
      if (event.ctrlKey && event.key.toLowerCase() === 'y') { event.preventDefault(); setHistory(redo(history)); setSelectedId(null); }
      if (event.key === 'Enter') { event.preventDefault(); void output('clipboard'); }
      if (event.ctrlKey) { const destination = ({ c: 'clipboard', s: 'file', t: 'pin' } as const)[event.key.toLowerCase() as 'c' | 's' | 't']; if (destination) { event.preventDefault(); void output(destination); } }
  };
  const sceneStyle = (rect: CaptureRect): React.CSSProperties => ({ left: rect.x * sx, top: rect.y * sy, width: rect.width * sx, height: rect.height * sy });
  return <main className="capture-overlay" ref={root} tabIndex={-1} onPointerDown={start} onPointerMove={move} onPointerUp={end}
    onPointerCancel={() => { gesture.current = null; setDraft(null); }}
    onContextMenu={(event) => { event.preventDefault(); if (busyRef.current) return; if (tool !== 'select' || selectedId || textEdit) { finishText(); setTool('select'); setSelectedId(null); gesture.current = null; setDraft(null); } else cancel(); }}
    onDoubleClick={(event) => {
      if ((event.target as HTMLElement).closest('[data-capture-ui]') || !selection) return;
      const item = hitAnnotation(history.present, eventPoint(event), 4 / sx);
      if (item?.tool === 'text') { setTextEdit({ id: item.id, point: item.points[0]!, text: item.text ?? '', color: item.color, width: item.width }); return; }
      void output('clipboard');
    }}
    onWheel={(event) => { if (tool !== 'select' && !(event.target as HTMLElement).closest('[data-capture-ui]')) setLineWidth((width) => clamp(width + (event.deltaY < 0 ? 1 : -1), 1, 32)); }}
    onKeyUp={(event) => { if (event.key === 'Alt') setAlt(false); }}
>
    <canvas className="capture-scene" ref={canvas} />
    {displayed ? <>
      <div className="capture-selection" style={sceneStyle(displayed)} aria-label="截图选区" />
      <div className="capture-size" style={{ left: clamp(displayed.x * sx, 4, viewport.width - 190), top: Math.max(4, displayed.y * sy - 28) }}>{displayed.width} × {displayed.height}{!selection && candidate?.label ? ` · ${candidate.label}` : ''}</div>
    </> : <div className="capture-shade" />}
    {selection && HANDLES.map((handle) => { const p = handlePoint(selection, handle); return <span key={handle} className={`capture-handle capture-handle--${handle}`} style={{ left: p.x * sx, top: p.y * sy }} />; })}
    {selectedItem && !textEdit && <div className="capture-object-outline" style={sceneStyle(annotationBounds(draft?.id === selectedItem.id ? draft : selectedItem))} />}
    {image && session && !textEdit && !busy && (tool === 'select' || alt) && <Magnifier image={image} point={point} screen={session.screen} />}
    {selection && !textEdit && <div className="capture-toolbar" data-capture-ui style={{ left: clamp(selection.x * sx, 12, Math.max(12, viewport.width - 790)), top: selection.y * sy + selection.height * sy + 66 < viewport.height ? selection.y * sy + selection.height * sy + 12 : Math.max(12, selection.y * sy - 62) }}>
      <div className="capture-tools">{TOOLS.map(([value, Icon, title]) => <button key={value} title={title} aria-label={title} aria-pressed={tool === value} disabled={busy} onClick={() => { setTool(value); setSelectedId(null); }}><Icon size={18} /></button>)}</div>
      <input type="color" value={color} aria-label="标注颜色" disabled={busy} onChange={(event) => setColor(event.target.value)} />
      <input type="number" className="capture-line-width" min={1} max={32} value={lineWidth} aria-label="线宽" disabled={busy} onChange={(event) => setLineWidth(clamp(Number(event.target.value) || 1, 1, 32))} />
      <button title="撤销 Ctrl+Z" aria-label="撤销" disabled={busy || !history.past.length} onClick={() => { setHistory(undo(history)); setSelectedId(null); }}><Undo2 size={18} /></button>
      <button title="重做 Ctrl+Y" aria-label="重做" disabled={busy || !history.future.length} onClick={() => { setHistory(redo(history)); setSelectedId(null); }}><Redo2 size={18} /></button>
      {selectedId && <button title="删除标注" aria-label="删除标注" disabled={busy} onClick={() => { setHistory(commit(history, history.present.filter((item) => item.id !== selectedId))); setSelectedId(null); }}><Trash2 size={18} /></button>}
      <span className="capture-divider" />
      <button title="保存 Ctrl+S" aria-label="保存 PNG" disabled={busy} onClick={() => void output('file')}><Download size={18} /></button>
      <button title="贴到屏幕 Ctrl+T" aria-label="创建贴图" disabled={busy} onClick={() => void output('pin')}><Pin size={18} /></button>
      <button title="问 AI" aria-label="问 AI" disabled={busy} onClick={() => void output('ask')}><MessageSquare size={18} /></button>
      <button title="复制 Enter / Ctrl+C" aria-label="复制截图" className="capture-primary" disabled={busy} onClick={() => void output('clipboard')}><Copy size={18} /></button>
      <button title="取消 Esc" aria-label="取消截图" disabled={busy} onClick={cancel}><X size={18} /></button>
    </div>}
    {textEdit && <div className="capture-text-editor" data-capture-ui style={{ left: clamp(textEdit.point.x * sx, 8, Math.max(8, viewport.width - 320)), top: clamp(textEdit.point.y * sy, 8, Math.max(8, viewport.height - 160)) }}>
      <textarea autoFocus aria-label="标注文字" maxLength={4000} value={textEdit.text} style={{ color: textEdit.color }} onChange={(event) => setTextEdit({ ...textEdit, text: event.target.value })} onKeyDown={(event) => { if (event.ctrlKey && event.key === 'Enter') { event.preventDefault(); finishText(); } }} />
      <button onClick={finishText}><Check size={16} /> 完成文字</button>
    </div>}
    {!selection && <div className="capture-help" data-capture-ui>拖动框选 · 单击选择窗口或控件 · Tab 切换层级 · Esc 取消</div>}
    {busy && <div className="capture-status" role="status">正在处理图片…</div>}
    {error && <div className="capture-error" role="alert" data-capture-ui>{error}<button onClick={() => setError('')} aria-label="关闭错误提示"><X size={14} /></button></div>}
  </main>;
}
