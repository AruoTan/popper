import { useEffect, useLayoutEffect, useRef, useState, type JSX } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { PinToolbarAction, PinToolbarState } from '../../shared/capture';
import { getErrorMessage } from '../lib/errors';
import { PinToolbar } from './PinToolbar';

export function PinToolbarApp(): JSX.Element {
  const [state, setState] = useState<PinToolbarState | null>(null), [error, setError] = useState('');
  const surface = useRef<HTMLDivElement>(null), keyboard = useRef<(event: KeyboardEvent) => void>(() => {}), alive = useRef(false);
  const queue = useRef(Promise.resolve());
  const action = (value: PinToolbarAction): void => {
    queue.current = queue.current.catch(() => {}).then(async () => {
      if (!alive.current) return;
      await invoke('capture_pin_toolbar_action', { action: value });
    }).catch((reason: unknown) => { if (alive.current) setError(getErrorMessage(reason, '无法操作贴图')); });
  };
  keyboard.current = (event) => {
    if (event.key === 'Escape' || (event.ctrlKey && event.key.toLowerCase() === 'w')) { event.preventDefault(); action({ action: 'close', destroy: event.shiftKey }); return; }
    if (event.target instanceof Element && event.target.closest('input')) return;
    if (event.ctrlKey) {
      const key = event.key.toLowerCase();
      const operation = key === 'c' ? 'copy' : key === 's' ? 'save' : key === 'y' || (key === 'z' && event.shiftKey) ? 'redo' : key === 'z' ? 'undo' : null;
      if (operation) { event.preventDefault(); action({ action: operation }); }
    }
    if (['Delete', 'Backspace'].includes(event.key)) { event.preventDefault(); action({ action: 'delete' }); }
  };
  useEffect(() => {
    alive.current = true; let active = true, dispose: (() => void) | undefined, revision = 0;
    void (async () => {
      try {
        const unlisten = await listen<PinToolbarState>('popper:pin-toolbar-state', (event) => { if (active) { revision++; setState(event.payload); } });
        if (!active) { unlisten(); return; } dispose = unlisten;
        const started = revision;
        const snapshot = await invoke<PinToolbarState>('capture_pin_toolbar_state');
        if (active && started === revision) setState(snapshot);
      } catch (reason) { if (active) setError(getErrorMessage(reason, '无法显示贴图工具条')); }
    })();
    const keyDown = (event: KeyboardEvent): void => keyboard.current(event);
    window.addEventListener('keydown', keyDown, true);
    return () => { active = false; alive.current = false; dispose?.(); window.removeEventListener('keydown', keyDown, true); };
  }, []);
  useLayoutEffect(() => {
    if (!state || !surface.current) return;
    let active = true, last = 0;
    const measure = (): void => {
      const height = Math.max(49, Math.ceil(surface.current!.getBoundingClientRect().height));
      if (height === last) return;
      last = height;
      void invoke('capture_pin_toolbar_ready', { height }).catch((reason: unknown) => { if (active) setError(getErrorMessage(reason, '无法定位贴图工具条')); });
    };
    measure();
    const observer = new ResizeObserver(measure); observer.observe(surface.current);
    return () => { active = false; observer.disconnect(); };
  }, [!!state]);
  return <div ref={surface} className="capture-pin-tools-surface" onContextMenu={(event) => event.preventDefault()}>
    {state && <PinToolbar state={state} onAction={(value) => { setError(''); action(value); }} />}
    {error && <div className="capture-pin-tools-error" role="alert">{error}</div>}
  </div>;
}
