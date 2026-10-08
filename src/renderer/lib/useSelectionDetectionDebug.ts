import { useEffect, useState } from "react";

/** Native process state, shared across windows and never stored in settings. */
export function useSelectionDetectionDebug() {
  const [enabled, setEnabled] = useState(false);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let current = true;
    let receivedEvent = false;
    const unsubscribe = window._popper_?.onSelectionDetectionDebugChanged?.((value) => {
      receivedEvent = true;
      if (current) {
        setEnabled(value);
        setReady(true);
      }
    });
    const read = window._popper_?.getSelectionDetectionDebugEnabled;
    if (read) {
      void read().then((value) => {
        if (current && !receivedEvent) {
          setEnabled(value);
          setReady(true);
        }
      }, () => { /* Keep diagnostics disabled if runtime state cannot be read. */ });
    }
    return () => { current = false; unsubscribe?.(); };
  }, []);

  return { enabled, ready, setEnabled };
}
