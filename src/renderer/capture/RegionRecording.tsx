import { useEffect, useRef, useState, type CSSProperties, type JSX } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Download, Pause, Play, Square, X, Mic, MicOff, MousePointer2 } from 'lucide-react';
import type { CaptureRect, RecordingProgress, RecordingSession, RecordingOptions } from '../../shared/capture';
import { getErrorMessage } from '../lib/errors';

type Operation = 'start' | 'pause' | 'stop' | 'save';
export function RegionRecording({ layout, screen, onCancel }: {
  layout: RecordingSession;
  screen: CaptureRect;
  onCancel: () => void;
}): JSX.Element {
  const [progress, setProgress] = useState<RecordingProgress>({ elapsedMs: 0, started: false, paused: false, stopped: false, error: null });
  const [options, setOptions] = useState<RecordingOptions>({ audio: false, cursor: false });
  const [busy, setBusy] = useState<Operation | null>(null), [error, setError] = useState(''), [statusError, setStatusError] = useState('');
  const operation = useRef<Operation | null>(null), generation = useRef(0);
  const mounted = useRef(true), cancelled = useRef(false), stopRef = useRef<() => void>(() => {});
  const cancel = (): void => { if (!cancelled.current) { cancelled.current = true; onCancel(); } };
  useEffect(() => {
    mounted.current = true;
    let alive = true, timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async (): Promise<void> => {
      if (!alive || cancelled.current) return;
      const revision = generation.current;
      try {
        const next = await invoke<RecordingProgress>('capture_record_status');
        if (!alive || cancelled.current) return;
        if (!operation.current && revision === generation.current) { setProgress(next); setStatusError(''); }
      } catch (e) {
        if (alive && !cancelled.current && !operation.current && revision === generation.current) setStatusError(getErrorMessage(e, '无法获取录屏状态，请停止并保存'));
      }
      if (alive && !cancelled.current) timer = setTimeout(() => void poll(), 250);
    };
    const keyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape' && !event.repeat) { event.preventDefault(); stopRef.current(); }
    };
    window.addEventListener('keydown', keyDown);
    void poll();
    return () => { alive = false; mounted.current = false; clearTimeout(timer); window.removeEventListener('keydown', keyDown); };
  }, []);
  const run = async (action: Operation): Promise<void> => {
    if (operation.current || cancelled.current) return;
    operation.current = action; generation.current++; setBusy(action); setError('');
    const current = (): boolean => mounted.current && !cancelled.current;
    try {
      if (action === 'start') {
        const next = await invoke<RecordingProgress>('capture_record_start', { options });
        if (current()) setProgress(next);
      } else if (action === 'pause') {
        const next = await invoke<RecordingProgress>('capture_record_pause', { paused: !progress.paused });
        if (current()) setProgress(next);
      } else {
        const stopped = await invoke<RecordingProgress>('capture_record_stop');
        if (!current()) return;
        setProgress(stopped);
        if (action === 'save') {
          setBusy('save');
          const saved = await invoke<boolean>('capture_record_save');
          if (saved || !current()) return;
        }
      }
    } catch (e) { if (current()) setError(getErrorMessage(e, action === 'start' ? '无法开始录屏' : action === 'pause' ? '无法切换暂停状态' : '无法保存录屏')); }
    if (current()) { operation.current = null; setBusy(null); }
  };
  stopRef.current = (): void => { if (!progress.started) cancel(); else if (!progress.stopped) void run('stop'); };
  const surface = progress.surface ?? { layout, screen };
  const position = (rect: CaptureRect): CSSProperties => ({ left: `${rect.x / surface.screen.width * 100}%`, top: `${rect.y / surface.screen.height * 100}%`, width: `${rect.width / surface.screen.width * 100}%`, height: `${rect.height / surface.screen.height * 100}%` });
  const seconds = Math.floor(progress.elapsedMs / 1000);
  const message = error || progress.error || statusError;
  const controlLabel = !progress.started ? '开始录制' : progress.paused ? '继续录制' : '暂停录制';
  const status = busy === 'stop' || (busy === 'save' && !progress.stopped) ? '正在停止…'
    : busy === 'start' ? '正在开始…' : busy === 'save' ? '正在保存…' : !progress.started ? '准备就绪' : progress.stopped ? '已停止，可保存' : progress.paused ? '已暂停' : '录制中';
  return <section className="capture-record-overlay" aria-label="区域录屏控制" aria-busy={busy !== null}>
    {surface.layout.selection.width > 0 && surface.layout.selection.height > 0 && <div className="capture-scroll-border capture-record-border" data-paused={!progress.started || progress.paused || progress.stopped} style={position(surface.layout.selection)} />}
    <div className="capture-toolbar-surface capture-toolbar capture-record-toolbar" role="toolbar" aria-label="区域录屏操作" data-capture-ui style={position(surface.layout.toolbar)}>
      <span className="capture-record-indicator" data-paused={!progress.started || progress.paused || progress.stopped} aria-hidden="true" />
      <span className="capture-record-time" role="timer" aria-label="录制时长">{Math.floor(seconds / 60).toString().padStart(2, '0')}:{(seconds % 60).toString().padStart(2, '0')}</span>
      <span className="capture-record-status" role={message ? 'alert' : 'status'} title={message || (progress.started ? 'Esc 停止并保留录制' : '点击开始录制 · Esc 退出')}>{message || status}</span>
      <button aria-label="录音开关" aria-pressed={options.audio} title={`${options.audio ? '录音已开启：系统声音和麦克风' : '录音已关闭'} · 开始前设置`} disabled={busy !== null || progress.started} onClick={() => setOptions({ ...options, audio: !options.audio })}>{options.audio ? <Mic size={16} /> : <MicOff size={16} />}</button>
      <button aria-label="鼠标开关" aria-pressed={options.cursor} title={`${options.cursor ? '录制鼠标已开启' : '录制鼠标已关闭'} · 开始前设置`} disabled={busy !== null || progress.started} onClick={() => setOptions({ ...options, cursor: !options.cursor })}><MousePointer2 size={16} /></button>
      {!progress.stopped && <button aria-label={controlLabel} title={controlLabel} disabled={busy !== null} onClick={() => void run(progress.started ? 'pause' : 'start')}>{!progress.started || progress.paused ? <Play size={16} /> : <Pause size={16} />}{!progress.started ? '开始' : progress.paused ? '继续' : '暂停'}</button>}
      <button aria-label={progress.stopped ? '保存 MP4' : '停止并保存'} title={!progress.started ? '开始录制后可完成并保存' : progress.stopped ? '保存 MP4' : '停止并保存 · Esc 仅停止'} disabled={busy !== null || !progress.started} onClick={() => void run('save')}>{progress.stopped ? <Download size={16} /> : <Square size={16} />}{progress.stopped ? '保存' : '完成'}</button>
      <button aria-label="丢弃录制" title={progress.started ? '丢弃本次录制' : '退出录屏准备'} onClick={cancel}><X size={16} /></button>
    </div>
  </section>;
}
