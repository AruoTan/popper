import { useEffect, useRef, useState, type CSSProperties, type JSX } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Mouse } from 'lucide-react';
import type { CaptureRect, CaptureOutput, ScrollCaptureProgress, ScrollCaptureSession } from '../../shared/capture';
import { getErrorMessage } from '../lib/errors';
import { prepareScrollPreview, type PreparedScrollPreview } from './scrollPreview';
import { ScrollCapturePreview } from './ScrollCapturePreview';
import { CaptureOutputToolbar } from './CaptureOutputToolbar';

const PREVIEW_INTERVAL_MS = 50;
const MESSAGES: Record<ScrollCaptureProgress['status'], string> = {
  appended: '滚动鼠标截取更多内容',
  tracked: '滚动鼠标截取更多内容',
  unchanged: '滚动鼠标截取更多内容',
  unmatched: '滚回已截取的位置后继续',
  limit: '已达到长图容量上限，请输出图片',
};
function position(rect: CaptureRect, screen: CaptureRect): CSSProperties {
  return { left: `${rect.x / screen.width * 100}%`, top: `${rect.y / screen.height * 100}%`, width: `${rect.width / screen.width * 100}%`, height: `${rect.height / screen.height * 100}%` };
}

export function ScrollCapture({ initial, screen, onAction, onCancel, error = '' }: {
  initial: ScrollCaptureSession;
  screen: CaptureRect;
  onAction: (action: CaptureOutput) => Promise<boolean>;
  onCancel: () => void;
  error?: string;
}): JSX.Element {
  const [finishing, setFinishing] = useState(false);
  const [message, setMessage] = useState(MESSAGES.unchanged);
  const [preview, setPreview] = useState<PreparedScrollPreview | null>(null);
  const [viewport, setViewport] = useState({ width: window.innerWidth, height: window.innerHeight });
  const ending = useRef(false), retry = useRef(false), inFlight = useRef<Promise<void>>(Promise.resolve());
  const mounted = useRef(true), cancelled = useRef(false);
  const previewUrls = useRef(new Set<string>()), decoding = useRef<AbortController | null>(null);
  const cancelRef = useRef<() => void>(() => {});

  useEffect(() => {
    mounted.current = true;
    const resize = (): void => setViewport({ width: window.innerWidth, height: window.innerHeight });
    const keyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') { event.preventDefault(); cancelRef.current(); }
    };
    window.addEventListener('resize', resize); window.addEventListener('keydown', keyDown);
    return () => {
      mounted.current = false;
      window.removeEventListener('resize', resize); window.removeEventListener('keydown', keyDown);
      decoding.current?.abort();
      for (const url of previewUrls.current) URL.revokeObjectURL(url);
      previewUrls.current.clear();
    };
  }, []);

  useEffect(() => {
    // Release the old URL only after React commits its replacement.
    for (const url of previewUrls.current) {
      if (url !== preview?.url) { URL.revokeObjectURL(url); previewUrls.current.delete(url); }
    }
  }, [preview]);
  useEffect(() => {
    let alive = true, revision = 0, timer: ReturnType<typeof setTimeout> | undefined;
    const poll = (): void => {
      if (!alive || ending.current) return;
      const started = performance.now();
      inFlight.current = (async () => {
        let limited = false;
        try {
          const next = await invoke<ScrollCaptureProgress>('capture_scroll_step');
          if (!alive || ending.current) return;
          setMessage(MESSAGES[next.status]); limited = next.status === 'limit';
          if (next.revision > revision) {
            const bytes = await invoke<ArrayBuffer>('capture_scroll_preview');
            if (!alive || ending.current) return;
            const controller = new AbortController(); decoding.current = controller;
            const snapshot = await prepareScrollPreview(bytes, controller.signal).finally(() => {
              if (decoding.current === controller) decoding.current = null;
            });
            if (!alive || ending.current) { URL.revokeObjectURL(snapshot.url); return; }
            revision = snapshot.progress.revision; previewUrls.current.add(snapshot.url);
            setPreview(snapshot);
          }
        } catch (captureError) {
          if (!alive || ending.current) return;
          setMessage(getErrorMessage(captureError, '无法获取页面，已保留当前长图'));
        }
        if (alive && !ending.current && !limited) timer = setTimeout(poll, Math.max(0, PREVIEW_INTERVAL_MS - (performance.now() - started)));
      })();
    };
    poll();
    return () => { alive = false; clearTimeout(timer); decoding.current?.abort(); };
  }, [initial]);

  const finish = async (action: CaptureOutput): Promise<void> => {
    if (cancelled.current || (ending.current && !retry.current)) return;
    ending.current = true; retry.current = false; decoding.current?.abort(); setFinishing(true);
    await inFlight.current;
    if (!mounted.current || cancelled.current) return;
    let finished = false;
    try { finished = await onAction(action); } catch { /* Keep the document available for another attempt. */ }
    if (!finished && mounted.current && !cancelled.current) {
      setFinishing(false); retry.current = true; setMessage('采集已停止，内容已保留，可重新输出');
    }
  };
  const cancel = (): void => {
    if (cancelled.current) return;
    cancelled.current = true; ending.current = true; decoding.current?.abort(); onCancel();
  };
  cancelRef.current = cancel;

  const layout = initial.layout;
  return <section className="capture-scroll-overlay" aria-label="滚动截图预览" aria-busy={finishing}
    onContextMenu={(event) => { event.preventDefault(); cancel(); }}>
    <div className="capture-scroll-border" style={position(layout.selection, screen)} data-interrupted={message === MESSAGES.unmatched} />
    <aside className="capture-scroll-panel" style={position(layout.panel, screen)} data-capture-ui>
      <header><Mouse size={22} aria-hidden="true" /><p role="status">{finishing ? '正在处理长图…' : message}</p><small>使用选区下方工具栏输出 · Esc 取消</small></header>
      <div className="capture-scroll-preview-area">
        {preview && <ScrollCapturePreview preview={preview}
          width={Math.max(0, layout.panel.width * viewport.width / screen.width - 26)}
          height={Math.max(0, layout.panel.height * viewport.height / screen.height - 134)} />}
      </div>
      {error && <div className="capture-scroll-error" role="alert">{error}</div>}
    </aside>
    <CaptureOutputToolbar label="滚动截图操作" style={position(layout.toolbar, screen)} busy={finishing}
      onOutput={(destination) => void finish(destination)} />
  </section>;
}
