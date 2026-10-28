import { useEffect, useRef, type PointerEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { CapturePoint } from '../../shared/capture';

interface Drag {
  pointerId: number;
  x: number;
  y: number;
  target: HTMLElement;
  anchor: CapturePoint | null;
  moving: boolean;
  pending: boolean;
  running: boolean;
}

export function usePinDrag(onError: (error: unknown) => void) {
  const current = useRef<Drag | null>(null);
  const report = useRef(onError);
  report.current = onError;

  const stop = (): void => {
    const drag = current.current;
    current.current = null;
    if (drag?.target.hasPointerCapture(drag.pointerId)) drag.target.releasePointerCapture(drag.pointerId);
  };
  useEffect(() => {
    window.addEventListener('blur', stop);
    return () => { window.removeEventListener('blur', stop); stop(); };
  }, []);

  const fail = (drag: Drag, error: unknown): void => {
    if (current.current !== drag) return;
    stop();
    report.current(error);
  };
  const move = async (drag: Drag): Promise<void> => {
    if (current.current !== drag || !drag.anchor || !drag.pending || drag.running) return;
    drag.running = true;
    drag.pending = false;
    try {
      // Sample the physical cursor natively; renderer screen coordinates change with DPI.
      const anchor = await invoke<CapturePoint | null>('capture_pin_drag', { anchor: drag.anchor });
      if (!anchor && current.current === drag) stop();
    } catch (error) { fail(drag, error); }
    finally {
      drag.running = false;
      if (current.current === drag && drag.pending) void move(drag);
    }
  };
  const onPointerDown = (event: PointerEvent<HTMLElement>): void => {
    stop();
    const drag: Drag = {
      pointerId: event.pointerId, x: event.clientX, y: event.clientY,
      target: event.currentTarget, anchor: null, moving: false, pending: false, running: false,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    current.current = drag;
    void invoke<CapturePoint | null>('capture_pin_drag', { anchor: null }).then((anchor) => {
      if (current.current !== drag) return;
      if (!anchor) { stop(); return; }
      drag.anchor = anchor;
      void move(drag);
    }).catch((error: unknown) => fail(drag, error));
  };
  const onPointerMove = (event: PointerEvent<HTMLElement>): void => {
    const drag = current.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    if (event.buttons !== 1) { stop(); return; }
    if (!drag.moving && Math.hypot(event.clientX - drag.x, event.clientY - drag.y) <= 4) return;
    drag.moving = true;
    drag.pending = true;
    void move(drag);
  };
  const end = (event: PointerEvent<HTMLElement>): void => {
    if (event.pointerId === current.current?.pointerId) stop();
  };
  return { stop, onPointerDown, onPointerMove, onPointerUp: end, onPointerCancel: end, onLostPointerCapture: end };
}
