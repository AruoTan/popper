import { useEffect, useRef, useState } from "react";
import type { DictionarySuggestion } from "../../shared";
import { getErrorMessage } from "../lib/errors";

// Rust owns recognition, the 250 ms debounce, and HTTP cancellation.
export function useTranslationSuggestions(
  sessionId: string,
  requestId: string | null,
  input: string,
  enabled: boolean,
) {
  const [items, setItems] = useState<DictionarySuggestion[]>([]);
  const [error, setError] = useState("");
  const version = useRef(Date.now() * 1000);
  useEffect(() => {
    const ticket = ++version.current;
    setItems([]);
    setError("");
    const suggest = window._popper_?.translationInputSuggestions;
    if (!requestId || !suggest || !enabled) return;
    void suggest(sessionId, requestId, ticket, input)
      .then((next) => {
        if (version.current === ticket) setItems(next);
      })
      .catch((cause: unknown) => {
        if (version.current === ticket && enabled && input.trim())
          setError(getErrorMessage(cause, "联想查询失败"));
      });
    return () => {
      const cancelled = ++version.current;
      void suggest(sessionId, requestId, cancelled, "").catch(() => {});
    };
  }, [sessionId, requestId, input, enabled]);
  return { items, error };
}
