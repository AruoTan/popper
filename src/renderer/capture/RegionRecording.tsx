import { useEffect, useRef, useState, type CSSProperties, type JSX } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Download, Square, X } from 'lucide-react';
import type { CaptureRect, RecordingProgress, RecordingSession } from '../../shared/capture';
import { getErrorMessage } from '../lib/errors';

export function RegionRecording({ layout, screen, onCancel }: {
  layout: RecordingSession;
  screen: CaptureRect;
  onCancel: () => void;
}): JSX.Element {
  const [progress, setProgress] = useState<RecordingProgress>({ elapsedMs: 0, stopped: false, error: null });
  const [busy, setBusy] = useState(false), [error, setError] = useState('');
  const saving = useRef(false), mounted = useRef(true), cancelled = useRef(false);
  const cancelRef = useRef(onCancel);
  cancelRef.current = onCancel;
  const cancel = (): void => { cancelled.current = true; cancelRef.current(); };
  useEffect(() => {
    mounted.current = true;
    let alive = true, timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async (): Promise<void> => {
      try {
        const next = await invoke<RecordingProgress>('capture_record_status');
        if (!alive || cancelled.current || saving.current) return;
        setProgress(next);
        if (!next.stopped) timer = setTimeout(() => void poll(), 250);
      } catch (e) { if (alive && !cancelled.current && !saving.current) setError(getErrorMessage(e, '无法获取录屏状态，请停止并保存')); }
    };
    const keyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') { event.preventDefault(); cancelled.current = true; cancelRef.current(); }
    };
    window.addEventListener('keydown', keyDown);
    void poll();
    return () => { alive = false; mounted.current = false; clearTimeout(timer); window.removeEventListener('keydown', keyDown); };
  }, []);
  const save = async (): Promise<void> => {
    if (saving.current || cancelled.current) return;
    saving.current = true; setBusy(true); setError('');
    try {
      const stopped = await invoke<RecordingProgress>('capture_record_stop');
      if (!mounted.current || cancelled.current) return;
      setProgress(stopped);
      const saved = await invoke<boolean>('capture_record_save');
      if (saved || !mounted.current) return;
    } catch (e) { if (mounted.current && !cancelled.current) setError(getErrorMessage(e, '无法保存录屏')); }
    if (mounted.current && !cancelled.current) { saving.current = false; setBusy(false); }
  };
  const position = (rect: CaptureRect): CSSProperties => ({ left: `${rect.x / screen.width * 100}%`, top: `${rect.y / screen.height * 100}%`, width: `${rect.width / screen.width * 100}%`, height: `${rect.height / screen.height * 100}%` });
  const seconds = Math.floor(progress.elapsedMs / 1000);
  return <section className="capture-scroll-overlay" aria-label="区域录屏控制" aria-busy={busy}>
    <div className="capture-scroll-border capture-record-border" style={position(layout.selection)} />
    <div className="capture-toolbar-surface capture-toolbar capture-record-toolbar" role="toolbar" aria-label="区域录屏操作" data-capture-ui style={position(layout.toolbar)}>
      <span className="capture-record-time" role="timer" aria-label="录制时长">{Math.floor(seconds / 60).toString().padStart(2, '0')}:{(seconds % 60).toString().padStart(2, '0')}</span>
      <span>{progress.stopped ? '已停止' : '录制中'} · 无音频</span>
      <button aria-label={progress.stopped ? '保存 MP4' : '停止并保存'} title={progress.stopped ? '保存 MP4' : '停止并保存'} disabled={busy} onClick={() => void save()}>{progress.stopped ? <Download size={16} /> : <Square size={16} />}</button>
      <button aria-label="取消录屏" title="取消录屏 Esc" onClick={cancel}><X size={16} /></button>
      {(error || progress.error) && <div className="capture-record-error" role="alert">{error || progress.error}</div>}
    </div>
  </section>;
}
