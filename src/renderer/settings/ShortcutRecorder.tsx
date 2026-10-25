import { Check, Keyboard, Pencil } from "lucide-react";
import { useEffect, useRef, useState, type JSX } from "react";

import { captureQuickAskShortcut, displayQuickAskShortcut, SHORTCUT_DISPLAY_NAMES } from "../quick-ask/shortcut";
import { getErrorMessage } from "../lib/errors";

const nativeRecordingQueues = new WeakMap<object, Promise<void>>();

interface ShortcutRecorderProps {
  value: string;
  disabled?: boolean;
  label?: string;
  id?: string;
  onChange: (shortcut: string) => void;
}

export function ShortcutRecorder({ value, disabled = false, label = "快捷提问快捷键", id = "quick-ask-shortcut", onChange }: ShortcutRecorderProps): JSX.Element {
  const [recording, setRecording] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [feedback, setFeedback] = useState<"idle" | "success" | "error">("idle");
  const [message, setMessage] = useState("");
  const [pressedKeys, setPressedKeys] = useState<string[]>([]);
  const generation = useRef(0);
  const active = useRef(false);
  const lastValue = useRef(value);
  const containerRef = useRef<HTMLDivElement>(null);

  // All recorders share one native queue so a delayed release from the
  // previous control cannot re-enable global shortcuts during a new recording.
  const setNativeRecording = (enabled: boolean): Promise<void> => {
    const api = window._popper_;
    const previous = nativeRecordingQueues.get(api) ?? Promise.resolve();
    const next = previous.catch(() => {}).then(async () => {
      await api.setQuickAskShortcutRecording?.(enabled);
    });
    nativeRecordingQueues.set(api, next);
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

  useEffect(() => {
    if (!recording && !preparing) return;
    const onPointerDown = (event: PointerEvent): void => {
      if (!containerRef.current?.contains(event.target as Node)) {
        finish();
        setFeedback("idle");
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [recording, preparing]);

  const keys = recording ? pressedKeys : value ? displayQuickAskShortcut(value).split(" + ") : [];
  const hint = preparing
    ? "准备录制…"
    : recording
      ? feedback === "error" ? message : "按下组合键，Esc 取消"
      : feedback === "success" ? "已更新，保存后生效" : feedback === "error" ? message : "点击修改快捷键";

  return (
    <div
      ref={containerRef}
      className={`shortcut-recorder ${recording ? "shortcut-recorder--recording" : ""} ${feedback === "error" ? "shortcut-recorder--error" : ""}`}
    >
      <button
        id={id}
        className="shortcut-recorder__control"
        type="button"
        aria-label={label}
        aria-describedby={`${id}-hint`}
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
            event.ctrlKey && "Ctrl",
            event.metaKey && "Win / ⌘",
            event.altKey && "Alt",
            event.shiftKey && "Shift",
            !/^(Control|Alt|Shift|Meta)(Left|Right)$/.test(event.code) &&
              (SHORTCUT_DISPLAY_NAMES[event.code] ?? event.code.replace(/^Key|^Digit/, "")),
          ].filter((key): key is string => Boolean(key)));
          if (/^(Control|Alt|Shift|Meta)(Left|Right)$/.test(event.code)) {
            setFeedback("idle");
            return;
          }
          const shortcut = captureQuickAskShortcut(event.nativeEvent);
          if (!shortcut) {
            setFeedback("error");
            setMessage("请使用修饰键组合，或 PrtSc、F1–F12 等标准快捷键");
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
            event.ctrlKey && "Ctrl",
            event.metaKey && "Win / ⌘",
            event.altKey && "Alt",
            event.shiftKey && "Shift",
            !/^(Control|Alt|Shift|Meta)(Left|Right)$/.test(event.code) &&
              (SHORTCUT_DISPLAY_NAMES[event.code] ?? event.code.replace(/^Key|^Digit/, "")),
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
          )) : <span className="shortcut-recorder__placeholder">{preparing ? "准备录制…" : recording ? "请按下快捷键" : "未设置"}</span>}
        </span>
        {recording ? <span className="shortcut-recorder__record-dot" aria-hidden="true" /> : <Pencil size={12} className="shortcut-recorder__edit" aria-hidden="true" />}
      </button>
      <span id={`${id}-hint`} className="shortcut-recorder__hint" role="status">
        {feedback === "success" && !recording && <Check size={12} aria-hidden="true" />}
        {hint}
      </span>
    </div>
  );
}
