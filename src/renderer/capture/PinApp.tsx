import { useEffect, useRef, useState, type JSX } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { PinnedImageState } from '../../shared/capture';
import { decodePng } from './drawing';
import { clamp } from './geometry';
import { getErrorMessage } from '../lib/errors';

export function PinApp(): JSX.Element {
  const [meta, setMeta] = useState<PinnedImageState | null>(null), [source, setSource] = useState(''), [menu, setMenu] = useState(false), [error, setError] = useState('');
  const current = useRef(meta), queue = useRef<Promise<unknown>>(Promise.resolve()), revision = useRef(0), dragging = useRef<{ x: number; y: number } | null>(null);
  current.current = meta;
  useEffect(() => {
    let active = true; let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const [state, bytes] = await Promise.all([invoke<PinnedImageState>('capture_pin_state'), invoke<ArrayBuffer>('capture_image')]);
        const image = await decodePng(bytes); if (!active) return;
        // Keep an owned URL until this window unmounts.
        const url = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
        if (!active) { URL.revokeObjectURL(url); return; }
        setSource(url); setMeta(state); current.current = state;
        if (image.naturalWidth !== state.width || image.naturalHeight !== state.height) throw new Error('贴图尺寸无效');
        await invoke('capture_show');
      } catch (e) { if (active) setError(getErrorMessage(e, '无法显示贴图')); }
    })();
    void listen<PinnedImageState>('popper:pin-updated', (event) => { if (active) { current.current = event.payload; setMeta(event.payload); } }).then((dispose) => { if (active) unlisten = dispose; else dispose(); });
    return () => { active = false; unlisten?.(); };
  }, []);
  useEffect(() => () => { if (source) URL.revokeObjectURL(source); }, [source]);
  const update = (patch: Partial<PinnedImageState>, x = .5, y = .5): void => {
    const previous = current.current; if (!previous) return;
    const next = { ...previous, ...patch }, sequence = ++revision.current;
    current.current = next; setMeta(next); setError('');
    queue.current = queue.current.catch(() => {}).then(() => invoke<PinnedImageState>('capture_pin_update', { update: { zoom: next.zoom, opacity: next.opacity, clickThrough: next.clickThrough, anchorX: x, anchorY: y } })).then((state) => { if (sequence === revision.current) { current.current = state; setMeta(state); } }).catch((e: unknown) => { setError(getErrorMessage(e, '无法调整贴图')); void invoke<PinnedImageState>('capture_pin_state').then((state) => { if (sequence === revision.current) { current.current = state; setMeta(state); } }).catch(() => {}); });
  };
  const close = (destroy = false): void => { setMenu(false); void invoke('capture_pin_close', { destroy }).catch((e: unknown) => setError(getErrorMessage(e, '无法关闭贴图'))); };
  return <main className="capture-pin" tabIndex={0} onContextMenu={(event) => { event.preventDefault(); void invoke('capture_pin_menu').catch(() => setMenu((visible) => !visible)); }} onDoubleClick={() => close()}
    onPointerDown={(event) => { if ((event.target as HTMLElement).closest('button')) return; if (event.button === 1) { event.preventDefault(); update({ zoom: 1, opacity: 1 }); } else if (event.button === 0) { dragging.current = { x: event.clientX, y: event.clientY }; setMenu(false); } }}
    onPointerMove={(event) => { const start = dragging.current; if (start && event.buttons === 1 && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 4) { dragging.current = null; void getCurrentWindow().startDragging().catch((e: unknown) => setError(getErrorMessage(e, '无法移动贴图'))); } }}
    onPointerUp={() => { dragging.current = null; }}
    onWheel={(event) => { event.preventDefault(); const state = current.current; if (!state) return; const amount = event.deltaY < 0 ? 1 : -1; if (event.ctrlKey) update({ opacity: Math.round(clamp(state.opacity + amount * .1, .1, 1) * 10) / 10 }); else update({ zoom: clamp(state.zoom * (amount > 0 ? 1.1 : 1 / 1.1), .1, 4) }, clamp(event.clientX / window.innerWidth, 0, 1), clamp(event.clientY / window.innerHeight, 0, 1)); }}
    onKeyDown={(event) => { if (event.key === 'Escape' || (event.ctrlKey && event.key.toLowerCase() === 'w')) { event.preventDefault(); close(event.shiftKey); } }}>
    {source && <img src={source} alt="浮动图片" draggable={false} onLoad={(event) => event.currentTarget.closest<HTMLElement>('main')?.focus()} />}
    {menu && <div className="capture-pin-menu" role="menu" onDoubleClick={(event) => event.stopPropagation()}>
      <span>{Math.round((meta?.zoom ?? 1) * 100)}% · 不透明度 {Math.round((meta?.opacity ?? 1) * 100)}%</span>
      <button role="menuitem" onClick={() => { update({ zoom: 1, opacity: 1 }); setMenu(false); }}>恢复原始大小和透明度</button>
      <button role="menuitem" onClick={() => { update({ clickThrough: true }); setMenu(false); }}>鼠标穿透（从托盘恢复）</button>
      <button role="menuitem" onClick={() => close()}>关闭 · 可恢复</button>
      <button role="menuitem" onClick={() => close(true)}>销毁图片</button>
    </div>}
    {error && <div className="capture-pin-error" role="alert">{error}</div>}
  </main>;
}
