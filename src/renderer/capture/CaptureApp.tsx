import { useEffect, useRef, useState, type JSX, type PointerEvent as ReactPointerEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, Copy, Download, MessageSquare, Pin, Redo2, Undo2, X, Trash2 } from 'lucide-react';
import type { CaptureAnnotation, CaptureCandidate, CaptureColorFormat, CaptureDocument, CaptureHitTest, CaptureOutput, CapturePoint, CaptureRect, CaptureSession, CaptureTool, ScrollCaptureSession } from '../../shared/capture';
import { getErrorMessage } from '../lib/errors';
import { annotationBounds, clamp, commit, constrainPoint, constrainedEnd, contains, emptyHistory, HANDLES, handlePoint, hitAnnotation, keyRect, moveRect, rectangle, redo, resizeHandle, resizeRect, undo, type Handle } from './geometry';
import { decodePng, pngBytes, renderScene, translateAnnotations } from './drawing';
import { createCaptureHover } from './hover';
import { createCaptureNavigation } from './navigation';
import { ScrollCapture } from './ScrollCapture';
import { ANNOTATION_TOOLS } from './annotationTools';
import { ScrollCaptureIcon } from './ScrollCaptureIcon';
import { Magnifier } from './Magnifier';

type Gesture = { mode: 'new' | 'move' | 'resize' | 'draw' | 'object'; start: CapturePoint; rect: CaptureRect | null; handle?: Handle; item?: CaptureAnnotation; moved: boolean };
type TextEdit = { id: string; point: CapturePoint; text: string; color: string; width: number };

export function CaptureApp(): JSX.Element {
  const [session, setSession] = useState<CaptureSession | null>(null), [image, setImage] = useState<HTMLImageElement | null>(null);
  const [selection, setSelection] = useState<CaptureRect | null>(null), [candidates, setCandidates] = useState<CaptureCandidate[]>([]), [candidateIndex, setCandidateIndex] = useState(0);
  const [point, setPoint] = useState<CapturePoint>({ x: 0, y: 0 }), [tool, setTool] = useState<CaptureTool>('select');
  const [history, setHistory] = useState(emptyHistory), [draft, setDraft] = useState<CaptureAnnotation | null>(null), [selectedId, setSelectedId] = useState<string | null>(null);
  const [textEdit, setTextEdit] = useState<TextEdit | null>(null), [color, setColor] = useState('#ff4545'), [lineWidth, setLineWidth] = useState(3);
  const [error, setError] = useState(''), [busy, setBusy] = useState(false), [viewport, setViewport] = useState({ width: window.innerWidth, height: window.innerHeight });
  const [scrollProgress, setScrollProgress] = useState<ScrollCaptureSession | null>(null);
  const [gestureMode, setGestureMode] = useState<Gesture['mode'] | null>(null);
  const [colorFormat, setColorFormat] = useState<CaptureColorFormat>('hex');
  const shiftColorToggle = useRef(false);
  const scrolling = scrollProgress !== null;
  const scrollingRef = useRef(false), scrollVersion = useRef<number | null>(null);
  const keyboard = useRef<(event: KeyboardEvent) => void>(() => {});
  const canvas = useRef<HTMLCanvasElement>(null), root = useRef<HTMLElement>(null), gesture = useRef<Gesture | null>(null), busyRef = useRef(false), version = useRef(0), generation = useRef(0);
  const hover = useRef<ReturnType<typeof createCaptureHover> | null>(null), mounted = useRef(true), selectedRef = useRef(selection);
  const pointerMoves = useRef(Promise.resolve());
  const pointerMoved = useRef<(point: CapturePoint) => void>(() => {});
  const cancelling = useRef(false), lifecycle = useRef(0), focusGeneration = useRef(0);
  const navigation = useRef<ReturnType<typeof createCaptureNavigation> | null>(null);
  selectedRef.current = selection;
  const setGesture = (next: Gesture | null): void => { gesture.current = next; setGestureMode(next?.mode ?? null); };
  const bounds: CaptureRect = { x: 0, y: 0, width: session?.screen.width ?? 1, height: session?.screen.height ?? 1 };
  const sx = viewport.width / bounds.width;
  const sy = viewport.height / bounds.height;
  const candidate = candidates[candidateIndex];
  const displayed = selection ?? candidate?.rect ?? null;
  const selectedItem = history.present.find((item) => item.id === selectedId);
  const displayedItems = history.present.filter((item) => item.id !== textEdit?.id && item.id !== (gesture.current?.mode === 'object' ? draft?.id : undefined));
  const magnifierActive = !!image && !!session && !textEdit && !busy && !scrolling && (!selection || gestureMode === 'new' || gestureMode === 'resize');

  const cancel = (): void => {
    if (cancelling.current) return;
    cancelling.current = true; lifecycle.current++; generation.current++;
    setGesture(null); setDraft(null);
    busyRef.current = true; setBusy(true);
    void invoke('capture_cancel').catch((e: unknown) => { if (mounted.current) { cancelling.current = false; busyRef.current = false; setBusy(false); setError(getErrorMessage(e, '无法结束截图')); } });
  };
  useEffect(() => {
    mounted.current = true; let alive = true, ready = false;
    let blurTimer: ReturnType<typeof setTimeout> | undefined;
    const clearBlur = (): void => { focusGeneration.current++; clearTimeout(blurTimer); blurTimer = undefined; };
    const focus = (): void => { clearBlur(); if (ready) root.current?.focus(); };
    void (async () => {
      try {
        const [meta, bytes] = await Promise.all([invoke<CaptureSession>('capture_session'), invoke<ArrayBuffer>('capture_image')]);
        const decoded = await decodePng(bytes); if (!alive || cancelling.current) return;
        setSession(meta); setImage(decoded); setColor(meta.color); setLineWidth(meta.lineWidth); if (meta.pointer) setPoint(meta.pointer);
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
        if (alive && !cancelling.current) { await invoke('capture_show'); if (alive && !cancelling.current) { ready = true; focus(); } }
      } catch (e) { if (alive && !cancelling.current) {
        const message = getErrorMessage(e, '无法初始化屏幕图像'); setError(message);
        void invoke('capture_fail', { message }).catch(() => {});
      } }
    })();
    const resize = (): void => setViewport({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener('resize', resize);
    const keyEvent = (event: KeyboardEvent): void => keyboard.current(event);
    window.addEventListener('keydown', keyEvent, true);
    window.addEventListener('keyup', keyEvent, true);
    window.addEventListener('focus', focus);
    const blur = (): void => {
      shiftColorToggle.current = false;
      clearBlur();
      if (!ready || busyRef.current || scrollingRef.current || !alive || cancelling.current) return;
      const revision = focusGeneration.current;
      // WebView focus transfers can emit blur while the capture frame stays foreground.
      blurTimer = setTimeout(() => {
        blurTimer = undefined;
        if (!alive || busyRef.current || cancelling.current || focusGeneration.current !== revision) return;
        void invoke<boolean>('capture_is_foreground').then((foreground) => {
          if (foreground === false && alive && !busyRef.current && !cancelling.current && focusGeneration.current === revision) cancel();
        }).catch(() => {});
      }, 120);
    };
    window.addEventListener('blur', blur);
    return () => { alive = false; mounted.current = false; clearBlur(); window.removeEventListener('resize', resize); window.removeEventListener('blur', blur); window.removeEventListener('keydown', keyEvent, true); window.removeEventListener('keyup', keyEvent, true); window.removeEventListener('focus', focus); generation.current++; lifecycle.current++; };
  }, []);
  useEffect(() => {
    if (!session || scrolling) return;
    const navigator = createCaptureNavigation(); navigation.current = navigator;
    const controller = createCaptureHover({
      targets: session.targets ?? [],
      active: () => mounted.current && !selectedRef.current && !gesture.current && !busyRef.current,
      request: (at) => invoke<CaptureHitTest>('capture_candidates', { x: at.x, y: at.y, generation: ++generation.current }),
      show: (items, { point, target, pending }) => {
        const next = navigator.update(items, point, target, { x: window.innerWidth / session.screen.width, y: window.innerHeight / session.screen.height }, pending);
        setCandidates(next.items); setCandidateIndex(next.index);
      },
    });
    hover.current = controller;
    if (session.pointer) controller.update(session.pointer);
    return () => { controller.destroy(); if (hover.current === controller) hover.current = null; if (navigation.current === navigator) navigation.current = null; };
  }, [session, scrolling]);
  useEffect(() => {
    if (!canvas.current || !image || scrolling) return;
    try { renderScene(canvas.current, image, bounds.width, bounds.height, draft ? [...displayedItems, draft] : displayedItems); }
    catch (e) { setError(getErrorMessage(e, '无法绘制标注')); }
  }, [image, history, draft, textEdit, bounds.width, bounds.height, scrolling]);

  useEffect(() => {
    if (!session || busy) return;
    const timer = window.setTimeout(() => { void invoke('capture_preferences', { color, lineWidth }).catch(() => {}); }, 250);
    return () => clearTimeout(timer);
  }, [session, color, lineWidth, busy]);

  const eventPoint = (event: { clientX: number; clientY: number }): CapturePoint => constrainPoint({ x: event.clientX / sx, y: event.clientY / sy }, bounds);
  const finishText = (): void => {
    if (!textEdit) return;
    const next = history.present.filter((item) => item.id !== textEdit.id);
    if (textEdit.text.trim()) next.push({ id: textEdit.id, tool: 'text', points: [textEdit.point], text: textEdit.text, color: textEdit.color, width: textEdit.width });
    setHistory(commit(history, next)); setSelectedId(textEdit.id); setTextEdit(null);
  };
  const output = async (destination: CaptureOutput): Promise<void> => {
    if (busyRef.current || scrollingRef.current || !selection || !image || textEdit) return;
    focusGeneration.current++;
    busyRef.current = true; setBusy(true); setError(''); setGesture(null); setDraft(null);
    const started = lifecycle.current;
    const current = (): boolean => mounted.current && !cancelling.current && lifecycle.current === started;
    try {
      await invoke('capture_preferences', { color, lineWidth }).catch(() => {});
      if (!current()) return;
      const revision = ++version.current;
      const original = await invoke<ArrayBuffer>('capture_crop', { rect: selection, version: revision, ...(destination === 'pin' ? { annotations: translateAnnotations(history.present, selection) } : {}) });
      if (!current()) return;
      let bytes: Uint8Array;
      if (destination === 'pin' || !history.present.length) bytes = new Uint8Array(original);
      else {
        const scene = document.createElement('canvas');
        renderScene(scene, image, bounds.width, bounds.height, history.present);
        const result = document.createElement('canvas'); result.width = selection.width; result.height = selection.height;
        const context = result.getContext('2d'); if (!context) throw new Error('无法创建导出画布');
        context.drawImage(scene, selection.x, selection.y, selection.width, selection.height, 0, 0, selection.width, selection.height);
        bytes = await pngBytes(result); scene.width = 0; scene.height = 0;
      }
      if (!current()) return;
      const finished = await invoke<boolean>('capture_export', bytes, { headers: { 'x-capture-version': String(revision), 'x-capture-output': destination } });
      if (!finished && current()) { busyRef.current = false; setBusy(false); root.current?.focus(); }
    } catch (e) { if (current()) { setError(getErrorMessage(e, '无法输出截图')); busyRef.current = false; setBusy(false); root.current?.focus(); } }
  };
  const beginScroll = async (): Promise<void> => {
    if (!selection || busyRef.current || scrollingRef.current || history.present.length) return;
    focusGeneration.current++; generation.current++;
    scrollingRef.current = true; busyRef.current = true; setBusy(true); setError('');
    setGesture(null); setDraft(null); setSelectedId(null); setTool('select');
    const started = lifecycle.current;
    try {
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      if (!mounted.current || cancelling.current || started !== lifecycle.current) return;
      const progress = await invoke<ScrollCaptureSession>('capture_scroll_begin', { rect: selection, version: ++version.current });
      if (!mounted.current || cancelling.current || started !== lifecycle.current) return;
      setScrollProgress(progress); busyRef.current = false; setBusy(false);
    } catch (e) {
      if (!mounted.current || cancelling.current || started !== lifecycle.current) return;
      scrollingRef.current = false; busyRef.current = false; setBusy(false);
      setError(getErrorMessage(e, '无法开始滚动截图')); root.current?.focus();
    }
  };
  const finishScroll = async (destination: CaptureOutput): Promise<boolean> => {
    if (!mounted.current || cancelling.current) return true;
    const started = lifecycle.current;
    const current = (): boolean => mounted.current && !cancelling.current && lifecycle.current === started;
    busyRef.current = true; setBusy(true); focusGeneration.current++;
    try {
      if (scrollVersion.current === null) {
        const doc = await invoke<CaptureDocument>('capture_scroll_finish');
        if (!current()) return true;
        scrollVersion.current = doc.version;
      }
      const bytes = await invoke<ArrayBuffer>('capture_image');
      if (!current()) return true;
      const finished = await invoke<boolean>('capture_export', new Uint8Array(bytes), { headers: { 'x-capture-version': String(scrollVersion.current), 'x-capture-output': destination } });
      if (!finished && current()) { busyRef.current = false; setBusy(false); }
      return finished || !current();
    } catch (e) {
      if (current()) {
        setError(getErrorMessage(e, '无法生成长图')); busyRef.current = false; setBusy(false);
      }
      return false;
    }
  };
  const start = (event: ReactPointerEvent<HTMLElement>): void => {
    shiftColorToggle.current = false;
    if (event.button !== 0 || busyRef.current || !session || !image || (event.target as HTMLElement).closest('[data-capture-ui]')) return;
    if (textEdit) { finishText(); return; }
    event.preventDefault(); root.current?.focus();
    const p = eventPoint(event); setPoint(p); setError(''); generation.current++;
    root.current?.setPointerCapture(event.pointerId);
    const handle = selection && resizeHandle(selection, p, { x: sx, y: sy });
    if (handle && selection) setGesture({ mode: 'resize', start: p, rect: selection, handle, moved: false });
    else if (tool !== 'select' && selection && contains(selection, p)) {
      if (tool === 'text') {
        if (root.current?.hasPointerCapture(event.pointerId)) root.current.releasePointerCapture(event.pointerId);
        setTextEdit({ id: crypto.randomUUID(), point: p, text: '', color, width: lineWidth }); return;
      }
      const item: CaptureAnnotation = { id: crypto.randomUUID(), tool, points: [p, p], color, width: lineWidth };
      setGesture({ mode: 'draw', start: p, rect: selection, item, moved: false }); setDraft(item);
    } else if (selection && contains(selection, p)) {
      const item = hitAnnotation(history.present, p, 4 / sx);
      if (item) { setGesture({ mode: 'object', start: p, rect: selection, item, moved: false }); setSelectedId(item.id); }
      else {
        setGesture({ mode: 'move', start: p, rect: selection, moved: false }); setSelectedId(null);
      }
    } else { setGesture({ mode: 'new', start: p, rect: null, moved: false }); setSelectedId(null); }
  };
  const updatePoint = (p: CapturePoint, shiftKey: boolean): void => {
    if (!session || busyRef.current) return;
    setPoint(p);
    const current = gesture.current;
    if (current && shiftKey) shiftColorToggle.current = false;
    if (!current) { if (!selection) hover.current?.update(p); return; }
    const dx = p.x - current.start.x, dy = p.y - current.start.y;
    if (Math.hypot(dx * sx, dy * sy) > 4) current.moved = true;
    if (current.mode === 'new' && current.moved) { setSelection(rectangle(current.start, p)); setCandidates([]); }
    if (current.mode === 'move' && current.rect) setSelection(moveRect(current.rect, dx, dy, bounds));
    if (current.mode === 'resize' && current.rect && current.handle) {
      const origin = handlePoint(current.rect, current.handle);
      setSelection(resizeRect(current.rect, current.handle, { x: origin.x + dx, y: origin.y + dy }, bounds));
    }
    if (current.mode === 'draw' && current.item) {
      const end = constrainPoint(constrainedEnd(current.start, p, current.item.tool, shiftKey), bounds);
      if (current.item.tool === 'pen') current.item = { ...current.item, points: [...current.item.points, p] };
      else current.item = { ...current.item, points: [current.start, end] };
      setDraft(current.item);
    }
    if (current.mode === 'object' && current.item && current.moved) setDraft({ ...current.item, points: current.item.points.map((point) => ({ x: point.x + dx, y: point.y + dy })) });
  };
  const move = (event: ReactPointerEvent<HTMLElement>): void => updatePoint(eventPoint(event), event.shiftKey);
  pointerMoved.current = (p): void => { if (!textEdit) updatePoint(p, false); };
  const end = (event: ReactPointerEvent<HTMLElement>): void => {
    const current = gesture.current; if (!current) return;
    setPoint(eventPoint(event));
    if (root.current?.hasPointerCapture(event.pointerId)) root.current.releasePointerCapture(event.pointerId);
    setGesture(null);
    if (current.mode === 'new' && !current.moved) {
      const p = eventPoint(event); const sequence = ++generation.current;
      if (candidate && contains(candidate.rect, p)) { setSelection(candidate.rect); setDraft(null); return; }
      void invoke<CaptureHitTest>('capture_candidates', { x: p.x, y: p.y, generation: sequence }).then(({ candidates: items }) => {
        if (mounted.current && !cancelling.current && generation.current === sequence && !gesture.current) setSelection(items[0]?.rect ?? { x: Math.min(p.x, bounds.width - 1), y: Math.min(p.y, bounds.height - 1), width: 1, height: 1 });
      }).catch(() => { if (mounted.current && !cancelling.current && generation.current === sequence) setSelection({ x: Math.min(p.x, bounds.width - 1), y: Math.min(p.y, bounds.height - 1), width: 1, height: 1 }); });
    }
    if (current.mode === 'new' && current.moved && (!selection?.width || !selection.height)) setSelection(null);
    if (current.mode === 'draw' && draft && (current.moved || draft.tool === 'pen')) { setHistory(commit(history, [...history.present, draft])); setSelectedId(draft.id); }
    if (current.mode === 'object' && current.moved && draft) setHistory(commit(history, history.present.map((item) => item.id === draft.id ? draft : item)));
    setDraft(null);
    if (selection && selection.width > 0 && selection.height > 0) void invoke('capture_select', { rect: selection, version: ++version.current }).catch(() => {});
  };
  keyboard.current = (event: KeyboardEvent): void => {
      if (event.type === 'keyup') {
        if (event.key === 'Shift') {
          if (shiftColorToggle.current && magnifierActive && !busyRef.current) setColorFormat((format) => format === 'hex' ? 'rgb' : 'hex');
          shiftColorToggle.current = false;
        }
        return;
      }
      if (event.key !== 'Shift') shiftColorToggle.current = false;
      if (event.key === 'Escape') { event.preventDefault(); cancel(); return; }
      if (scrollingRef.current) return;
      if (event.key === 'Tab' && !selection && !textEdit && !event.ctrlKey && !event.altKey && !event.metaKey) {
        event.preventDefault();
        if (!busyRef.current) {
          const next = navigation.current?.cycle(event.shiftKey ? -1 : 1);
          if (next) { setCandidates(next.items); setCandidateIndex(next.index); }
        }
        return;
      }
      if (busyRef.current) { if (event.key === 'Tab') event.preventDefault(); return; }
      if (event.target instanceof Element && event.target.closest('input, textarea')) return;
      if (event.key === 'Shift' && magnifierActive && !event.ctrlKey && !event.altKey && !event.metaKey) {
        if (!event.repeat) shiftColorToggle.current = true;
        return;
      }
      if (event.ctrlKey && event.key.toLowerCase() === 'c' && !event.shiftKey && !event.altKey && !event.metaKey && magnifierActive) {
        event.preventDefault();
        if (!event.repeat) {
          setError('');
          const started = lifecycle.current;
          void invoke('capture_copy_color', { x: point.x, y: point.y, format: colorFormat }).catch((e: unknown) => {
            if (mounted.current && !cancelling.current && lifecycle.current === started) setError(getErrorMessage(e, '无法复制色值'));
          });
        }
        return;
      }
      if (event.key.startsWith('Arrow') && session && image && !textEdit) {
        if (event.altKey || event.metaKey) return;
        event.preventDefault();
        if (event.ctrlKey || event.shiftKey) {
          if (selection) setSelection(keyRect(selection, event.key, event.ctrlKey, event.shiftKey, bounds));
        } else {
          const dx = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
          const dy = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0;
          pointerMoves.current = pointerMoves.current.then(async () => {
            if (!mounted.current || busyRef.current) return;
            const next = await invoke<CapturePoint>('capture_move_pointer', { dx, dy });
            if (mounted.current && !busyRef.current) pointerMoved.current(next);
          }).catch((e: unknown) => { if (mounted.current && !busyRef.current) setError(getErrorMessage(e, '无法移动鼠标')); });
        }
        return;
      }
      if ((event.key === 'Delete' || event.key === 'Backspace') && selectedId) { event.preventDefault(); setHistory(commit(history, history.present.filter((item) => item.id !== selectedId))); setSelectedId(null); }
      if (event.ctrlKey && event.key.toLowerCase() === 'z') { event.preventDefault(); setHistory(event.shiftKey ? redo(history) : undo(history)); setSelectedId(null); }
      if (event.ctrlKey && event.key.toLowerCase() === 'y') { event.preventDefault(); setHistory(redo(history)); setSelectedId(null); }
      if (event.key === 'Enter') { event.preventDefault(); void output('clipboard'); }
      if (event.ctrlKey) { const destination = ({ c: 'clipboard', s: 'file', t: 'pin' } as const)[event.key.toLowerCase() as 'c' | 's' | 't']; if (destination) { event.preventDefault(); void output(destination); } }
  };
  const adjusting = gestureMode === 'new' || gestureMode === 'move' || gestureMode === 'resize';
  const handle = selection && resizeHandle(selection, point, { x: sx, y: sy });
  const cursor = gestureMode === 'resize' ? `${gesture.current?.handle}-resize`
    : gestureMode === 'move' || gestureMode === 'object' ? 'move'
    : gestureMode === 'new' || gestureMode === 'draw' || !selection ? 'crosshair'
    : handle ? `${handle}-resize`
    : !contains(selection, point) ? 'default' : tool === 'select' ? 'move' : tool === 'text' ? 'text' : 'crosshair';
  const sceneStyle = (rect: CaptureRect): React.CSSProperties => ({ left: rect.x * sx, top: rect.y * sy, width: rect.width * sx, height: rect.height * sy });
  if (scrollProgress && session) return <main className="capture-scroll-surface"><ScrollCapture initial={scrollProgress} screen={session.screen} onAction={finishScroll} onCancel={cancel} error={error} /></main>;
  return <main className="capture-overlay" style={{ cursor }} ref={root} tabIndex={-1} onPointerDown={start} onPointerMove={move} onPointerUp={end}
    onPointerCancel={() => { setGesture(null); setDraft(null); }}
    onContextMenu={(event) => { event.preventDefault(); if (busyRef.current) return; if (tool !== 'select' || selectedId || textEdit) { finishText(); setTool('select'); setSelectedId(null); setGesture(null); setDraft(null); } else cancel(); }}
    onDoubleClick={(event) => {
      if ((event.target as HTMLElement).closest('[data-capture-ui]') || !selection) return;
      const item = hitAnnotation(history.present, eventPoint(event), 4 / sx);
      if (item?.tool === 'text') { setTextEdit({ id: item.id, point: item.points[0]!, text: item.text ?? '', color: item.color, width: item.width }); return; }
      void output('clipboard');
    }}
    onWheel={(event) => { if (tool !== 'select' && !(event.target as HTMLElement).closest('[data-capture-ui]')) setLineWidth((width) => clamp(width + (event.deltaY < 0 ? 1 : -1), 1, 32)); }}
>
    <canvas className="capture-scene" ref={canvas} />
    {displayed ? <>
      <div className="capture-selection" style={sceneStyle(displayed)} aria-label="截图选区" />
      <div className="capture-size" style={{ left: clamp(displayed.x * sx, 4, viewport.width - 190), top: Math.max(4, displayed.y * sy - 28) }}>{displayed.width} × {displayed.height}{!selection && candidate?.label ? ` · ${candidate.label}` : ''}</div>
    </> : <div className="capture-shade" />}
    {selection && HANDLES.map((handle) => { const p = handlePoint(selection, handle); return <span key={handle} className={`capture-handle capture-handle--${handle}`} style={{ left: p.x * sx, top: p.y * sy }} />; })}
    {selectedItem && !textEdit && <div className="capture-object-outline" style={sceneStyle(annotationBounds(draft?.id === selectedItem.id ? draft : selectedItem))} />}
    {magnifierActive && image && session && <Magnifier image={image} point={point} screen={session.screen} viewport={viewport} format={colorFormat} />}
    {selection && !adjusting && !textEdit && <div className="capture-toolbar-surface capture-toolbar" role="toolbar" aria-label="截图工具栏" data-capture-ui style={{ left: clamp(selection.x * sx, 12, Math.max(12, viewport.width - 790)), top: selection.y * sy + selection.height * sy + 66 < viewport.height ? selection.y * sy + selection.height * sy + 12 : Math.max(12, selection.y * sy - 62) }}>
      <div className="capture-tools">{ANNOTATION_TOOLS.map(([value, Icon, title]) => <button key={value} title={title} aria-label={title} aria-pressed={tool === value} disabled={busy} onClick={() => { setTool(value); setSelectedId(null); }}><Icon size={16} /></button>)}</div>
      <input type="color" value={color} aria-label="标注颜色" disabled={busy} onChange={(event) => setColor(event.target.value)} />
      <input type="number" min={1} max={32} value={lineWidth} aria-label="线宽" disabled={busy} onChange={(event) => setLineWidth(clamp(Number(event.target.value) || 1, 1, 32))} />
      <button title="撤销 Ctrl+Z" aria-label="撤销" disabled={busy || !history.past.length} onClick={() => { setHistory(undo(history)); setSelectedId(null); }}><Undo2 size={16} /></button>
      <button title="重做 Ctrl+Y" aria-label="重做" disabled={busy || !history.future.length} onClick={() => { setHistory(redo(history)); setSelectedId(null); }}><Redo2 size={16} /></button>
      {selectedId && <button title="删除标注" aria-label="删除标注" disabled={busy} onClick={() => { setHistory(commit(history, history.present.filter((item) => item.id !== selectedId))); setSelectedId(null); }}><Trash2 size={16} /></button>}
      <span className="capture-divider" />
      <button title={history.present.length ? '已有标注，无法开始滚动截图' : '滚动截图'} aria-label="滚动截图" disabled={busy || history.present.length > 0 || selection.width < 32 || selection.height < 96} onClick={() => void beginScroll()}><ScrollCaptureIcon /></button>
      <button title="保存 Ctrl+S" aria-label="保存 PNG" disabled={busy} onClick={() => void output('file')}><Download size={16} /></button>
      <button title="贴到屏幕 Ctrl+T" aria-label="创建贴图" disabled={busy} onClick={() => void output('pin')}><Pin size={16} /></button>
      <button title="问 AI" aria-label="问 AI" disabled={busy} onClick={() => void output('ask')}><MessageSquare size={16} /></button>
      <button title="复制 Enter / Ctrl+C" aria-label="复制截图" disabled={busy} onClick={() => void output('clipboard')}><Copy size={16} /></button>
      <button title="取消 Esc" aria-label="取消截图" disabled={busy} onClick={cancel}><X size={16} /></button>
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
