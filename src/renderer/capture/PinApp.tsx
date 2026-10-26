import { useEffect, useRef, useState, type CSSProperties, type JSX } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { PinnedImageState } from '../../shared/capture';
import { decodePng } from './drawing';
import { clamp } from './geometry';
import { getErrorMessage } from '../lib/errors';
import { usePinDrag } from './usePinDrag';

export function PinApp(): JSX.Element {
  const [meta, setMeta] = useState<PinnedImageState | null>(null), [source, setSource] = useState(''), [menu, setMenu] = useState(false), [error, setError] = useState('');
  const [shadowEpoch, setShadowEpoch] = useState(0);
  const alive = useRef(false), showing = useRef(false);
  const root = useRef<HTMLElement>(null), closing = useRef(false);
  const keyboard = useRef<(event: KeyboardEvent) => void>(() => {});
  const current = useRef(meta), queue = useRef<Promise<unknown>>(Promise.resolve()), revision = useRef(0);
  const drag = usePinDrag((e) => setError(getErrorMessage(e, '无法移动贴图')));
  current.current = meta;
  useEffect(() => {
    alive.current = true;
    let active = true; let unlisten: (() => void) | undefined, unlistenShown: (() => void) | undefined;
    void (async () => {
      try {
        const [state, bytes] = await Promise.all([invoke<PinnedImageState>('capture_pin_state'), invoke<ArrayBuffer>('capture_image')]);
        const image = await decodePng(bytes); if (!active) return;
        if (image.naturalWidth !== state.width || image.naturalHeight !== state.height) throw new Error('贴图尺寸无效');
        // Keep an owned URL until this window unmounts.
        const url = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
        setSource(url); setMeta(state); current.current = state;
      } catch (e) { if (active) setError(getErrorMessage(e, '无法显示贴图')); }
    })();
    void listen<PinnedImageState>('popper:pin-updated', (event) => { if (active) { current.current = event.payload; setMeta(event.payload); } }).then((dispose) => { if (active) unlisten = dispose; else dispose(); });
    void listen('popper:pin-shown', () => { if (active) { closing.current = false; setShadowEpoch((epoch) => epoch + 1); } }).then((dispose) => { if (active) unlistenShown = dispose; else dispose(); });
    const keyDown = (event: KeyboardEvent): void => keyboard.current(event);
    const focus = (): void => root.current?.focus();
    window.addEventListener('keydown', keyDown, true);
    window.addEventListener('focus', focus);
    return () => { active = false; alive.current = false; unlisten?.(); unlistenShown?.(); window.removeEventListener('keydown', keyDown, true); window.removeEventListener('focus', focus); };
  }, []);
  useEffect(() => () => { if (source) URL.revokeObjectURL(source); }, [source]);
  const update = (patch: Partial<PinnedImageState>, x = .5, y = .5): void => {
    const previous = current.current; if (!previous) return;
    const next = { ...previous, ...patch }, sequence = ++revision.current;
    current.current = next; setMeta(next); setError('');
    queue.current = queue.current.catch(() => {}).then(() => invoke<PinnedImageState>('capture_pin_update', { update: { zoom: next.zoom, opacity: next.opacity, clickThrough: next.clickThrough, anchorX: x, anchorY: y } })).then((state) => { if (sequence === revision.current) { current.current = state; setMeta(state); } }).catch((e: unknown) => { setError(getErrorMessage(e, '无法调整贴图')); void invoke<PinnedImageState>('capture_pin_state').then((state) => { if (sequence === revision.current) { current.current = state; setMeta(state); } }).catch(() => {}); });
  };
  const close = (destroy = false): void => {
    if (closing.current) return;
    drag.stop();
    closing.current = true; setMenu(false);
    void invoke('capture_pin_close', { destroy }).catch((e: unknown) => { if (alive.current) { closing.current = false; setError(getErrorMessage(e, '无法关闭贴图')); } });
  };
  keyboard.current = (event): void => {
    if (event.key === 'Escape' || (event.ctrlKey && event.key.toLowerCase() === 'w')) { event.preventDefault(); close(event.shiftKey); }
  };
  const inset = meta?.shadowInset ?? 0;
  const frameWidth = Math.max(1, Math.round((meta?.width ?? 1) * (meta?.zoom ?? 1))) + inset * 2;
  const frameHeight = Math.max(1, Math.round((meta?.height ?? 1) * (meta?.zoom ?? 1))) + inset * 2;
  return <div className="capture-pin-frame" style={{ '--pin-shadow-unit': `calc(100vw / ${frameWidth})` } as CSSProperties}>
    <main ref={root} className="capture-pin" style={{ left: `${inset / frameWidth * 100}%`, right: `${inset / frameWidth * 100}%`, top: `${inset / frameHeight * 100}%`, bottom: `${inset / frameHeight * 100}%` }} tabIndex={0} onContextMenu={(event) => { event.preventDefault(); drag.stop(); void invoke('capture_pin_menu').catch(() => setMenu((visible) => !visible)); }} onDoubleClick={() => close()}
    onPointerDown={(event) => { if ((event.target as HTMLElement).closest('button')) return; if (event.button === 1) { event.preventDefault(); drag.stop(); update({ zoom: 1, opacity: 1 }); } else if (event.button === 0) { drag.onPointerDown(event); setMenu(false); } }}
    onPointerMove={drag.onPointerMove} onPointerUp={drag.onPointerUp} onPointerCancel={drag.onPointerCancel} onLostPointerCapture={drag.onLostPointerCapture}
    onWheel={(event) => { event.preventDefault(); const state = current.current; if (!state) return; const amount = event.deltaY < 0 ? 1 : -1; const bounds = event.currentTarget.getBoundingClientRect(); if (event.ctrlKey) update({ opacity: Math.round(clamp(state.opacity + amount * .1, .1, 1) * 10) / 10 }); else update({ zoom: clamp(state.zoom * (amount > 0 ? 1.1 : 1 / 1.1), .1, 4) }, clamp((event.clientX - bounds.left) / bounds.width, 0, 1), clamp((event.clientY - bounds.top) / bounds.height, 0, 1)); }}
    >
    {shadowEpoch > 0 && <span key={shadowEpoch} className="capture-pin-shadow" aria-hidden="true" />}
    {source && <img src={source} alt="浮动图片" draggable={false} onLoad={(event) => {
      if (showing.current) return;
      const main = event.currentTarget.closest<HTMLElement>('main'); showing.current = true;
      void invoke('capture_show').then(() => { if (alive.current) { setShadowEpoch((epoch) => epoch + 1); main?.focus(); } }).catch((e: unknown) => { if (alive.current) { showing.current = false; setError(getErrorMessage(e, '无法显示贴图')); } });
    }} />}
    {menu && <div className="capture-pin-menu" role="menu" onDoubleClick={(event) => event.stopPropagation()}>
      <span>{Math.round((meta?.zoom ?? 1) * 100)}% · 不透明度 {Math.round((meta?.opacity ?? 1) * 100)}%</span>
      <button role="menuitem" onClick={() => { update({ zoom: 1, opacity: 1 }); setMenu(false); }}>恢复原始大小和透明度</button>
      <button role="menuitem" onClick={() => { update({ clickThrough: true }); setMenu(false); }}>鼠标穿透（从托盘恢复）</button>
      <button role="menuitem" onClick={() => close()}>关闭 · 可恢复</button>
      <button role="menuitem" onClick={() => close(true)}>销毁图片</button>
    </div>}
    {error && <div className="capture-pin-error" role="alert">{error}</div>}
  </main></div>;
}
