import { useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { PinToolbarAction, PinToolbarState } from '../../shared/capture';

export function usePinToolbar(onAction: (action: PinToolbarAction) => void, onError: (error: unknown) => void) {
  const handlers = useRef({ onAction, onError }); handlers.current = { onAction, onError };
  const lifecycle = useRef(0), pending = useRef<PinToolbarState | null>(null), running = useRef(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    const generation = ++lifecycle.current; let active = true;
    const disposers: (() => void)[] = [];
    for (const [event, callback] of [
      ['popper:pin-toolbar-action', (payload: unknown) => handlers.current.onAction(payload as PinToolbarAction)],
      ['popper:pin-toolbar-error', (payload: unknown) => handlers.current.onError(payload)],
    ] as const) {
      void listen(event, (message) => { if (active) callback(message.payload); }).then((dispose) => { if (active) disposers.push(dispose); else dispose(); }).catch((error: unknown) => { if (active) handlers.current.onError(error); });
    }
    return () => { active = false; alive.current = false; if (lifecycle.current === generation) lifecycle.current++; pending.current = null; disposers.forEach((dispose) => dispose()); };
  }, []);
  return useCallback((state: PinToolbarState): void => {
    pending.current = state;
    if (running.current) return;
    const flush = async (): Promise<void> => {
      running.current = true;
      try {
        while (pending.current && alive.current) {
          const generation = lifecycle.current;
          const snapshot = pending.current; pending.current = null;
          try { await invoke('capture_pin_toolbar_publish', { snapshot }); }
          catch (error) { if (generation === lifecycle.current) handlers.current.onError(error); }
        }
      } finally { running.current = false; }
    };
    void flush();
  }, []);
}
