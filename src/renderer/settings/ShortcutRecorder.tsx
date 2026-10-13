import { Check, Keyboard, Pencil } from "lucide-react";
import { useEffect, useRef, useState, type JSX } from "react";

import { captureQuickAskShortcut, displayQuickAskShortcut } from "../quick-ask/shortcut";
import { getErrorMessage } from "../lib/errors";

interface ShortcutRecorderProps {
  value: string;
  disabled?: boolean;
  onChange: (shortcut: string) => void;
}

export function ShortcutRecorder({ value, disabled = false, onChange }: ShortcutRecorderProps): JSX.Element {
  const [recording, setRecording] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [feedback, setFeedback] = useState<"idle" | "success" | "error">("idle");
  const [message, setMessage] = useState("");
  const [pressedKeys, setPressedKeys] = useState<string[]>([]);
  const generation = useRef(0);
  const nativeQueue = useRef<Promise<void>>(Promise.resolve());
  const active = useRef(false);
  const lastValue = useRef(value);

  // Serialize begin/end so a delayed native acknowledgement cannot leave
  // the global shortcut suspended after cancellation or component unmount.
  const setNativeRecording = (enabled: boolean): Promise<void> => {
    const next = nativeQueue.current.catch(() => {}).then(async () => {
      await window._popper_.setQuickAskShortcutRecording?.(enabled);
    });
    nativeQueue.current = next;
    return next;
  };

  const finish = (): void => {
    generation.current += 1;
    active.current = false;
    setRecording(false);
    setPreparing(false);
    setPressedKeys([]);
    void setNativeRecording(false).catch(() => {});
  };

  useEffect(() => {
    const onWindowBlur = (): void => {
      if (!active.current) return;
      finish();
      setFeedback("idle");
    };
    window.addEventListener("blur", onWindowBlur);
    return () => {
      window.removeEventListener("blur", onWindowBlur);
      generation.current += 1;
      active.current = false;
      void setNativeRecording(false).catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (disabled && active.current) finish();
  }, [disabled]);

  useEffect(() => {
    if (value !== lastValue.current) {
      if (active.current) finish();
      setFeedback("idle");
      lastValue.current = value;
    }
  }, [value]);

  const begin = async (): Promise<void> => {
    if (disabled || active.current) return;
    active.current = true;
    const current = ++generation.current;
    setPreparing(true);
    setFeedback("idle");
    setMessage("");
    try {
      await setNativeRecording(true);
      if (current !== generation.current) return;
      setPreparing(false);
      setRecording(true);
    } catch (error: unknown) {
      if (current !== generation.current) return;
      finish();
      setFeedback("error");
      setMessage(getErrorMessage(error, "无法开始录制，请重试"));
    }
  };

  const keys = recording ? pressedKeys : displayQuickAskShortcut(value).split(" + ");
  const hint = preparing
    ? "准备录制…"
    : recording
      ? feedback === "error" ? message : "按下组合键，Esc 取消"
      : feedback === "success" ? "已更新，保存后生效" : feedback === "error" ? message : "点击修改快捷键";

  return (
    <div className={`shortcut-recorder ${recording ? "shortcut-recorder--recording" : ""} ${feedback === "error" ? "shortcut-recorder--error" : ""}`}>
      <button
        id="quick-ask-shortcut"
        className="shortcut-recorder__control"
        type="button"
        aria-label="快捷提问快捷键"
        aria-describedby="quick-ask-shortcut-hint"
        aria-pressed={recording}
        aria-busy={preparing}
        disabled={disabled}
        onClick={() => {
          if (recording) { finish(); setFeedback("idle"); }
          else void begin();
        }}
        onBlur={() => {
          if (active.current) { finish(); setFeedback("idle"); }
        }}
        onKeyDown={(event) => {
          if (preparing && (event.key === "Escape" || event.key === "Tab")) {
            if (event.key === "Escape") event.preventDefault();
            finish();
            setFeedback("idle");
            return;
          }
          if (preparing && event.key !== "Tab") { event.preventDefault(); return; }
          if (!recording) return;
          if (event.key === "Tab") { finish(); setFeedback("idle"); return; }
          event.preventDefault();
          event.stopPropagation();
          if (event.key === "Escape") { finish(); setFeedback("idle"); return; }
          if (event.nativeEvent.isComposing || event.repeat) return;
          setPressedKeys([
            event.ctrlKey && "Ctrl", event.altKey && "Alt",
            event.shiftKey && "Shift", event.metaKey && "Win / ⌘",
          ].filter((key): key is string => Boolean(key)));
          if (/^(Control|Alt|Shift|Meta)(Left|Right)$/.test(event.code)) {
            setFeedback("idle");
            return;
          }
          const shortcut = captureQuickAskShortcut(event.nativeEvent);
          if (!shortcut) {
            setFeedback("error");
            setMessage("请使用修饰键 + 字母、数字、空格或 F1–F24");
            return;
          }
          lastValue.current = shortcut;
          onChange(shortcut);
          finish();
          setFeedback("success");
        }}
        onKeyUp={(event) => {
          if (!recording) return;
          setPressedKeys([
            event.ctrlKey && "Ctrl", event.altKey && "Alt",
            event.shiftKey && "Shift", event.metaKey && "Win / ⌘",
          ].filter((key): key is string => Boolean(key)));
        }}
      >
        <Keyboard size={16} className="shortcut-recorder__keyboard" aria-hidden="true" />
        <span className="shortcut-recorder__keys">
          {keys.length ? keys.map((key, index) => (
            <span className="shortcut-recorder__key" key={key}>
              {index > 0 && <span className="shortcut-recorder__plus" aria-hidden="true">+</span>}
              <kbd>{key}</kbd>
            </span>
          )) : <span className="shortcut-recorder__placeholder">{preparing ? "准备录制…" : "请按下快捷键"}</span>}
        </span>
        {recording ? <span className="shortcut-recorder__record-dot" aria-hidden="true" /> : <Pencil size={12} className="shortcut-recorder__edit" aria-hidden="true" />}
      </button>
      <span id="quick-ask-shortcut-hint" className="shortcut-recorder__hint" role="status">
        {feedback === "success" && !recording && <Check size={12} aria-hidden="true" />}
        {hint}
      </span>
    </div>
  );
}
