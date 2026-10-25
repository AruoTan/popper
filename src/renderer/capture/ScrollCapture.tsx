import { useEffect, useRef, useState, type CSSProperties, type JSX } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Mouse } from 'lucide-react';
import type { CaptureRect, CaptureOutput, ScrollCaptureProgress, ScrollCaptureSession } from '../../shared/capture';
import { getErrorMessage } from '../lib/errors';
import { decodeScrollPreview } from './scrollPreview';
import { CaptureOutputToolbar } from './CaptureOutputToolbar';

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
  initial: ScrollCaptureSession | null;
  screen: CaptureRect;
  onAction: (action: CaptureOutput) => Promise<boolean>;
  onCancel: () => void;
  error?: string;
}): JSX.Element {
  const [finishing, setFinishing] = useState(false);
  const [message, setMessage] = useState(MESSAGES.unchanged);
  const [preview, setPreview] = useState<{ url: string; progress: ScrollCaptureProgress } | null>(null);
  const [viewport, setViewport] = useState({ width: window.innerWidth, height: window.innerHeight });
  const ending = useRef(false), retry = useRef(false), inFlight = useRef<Promise<void>>(Promise.resolve());
  const mounted = useRef(true), cancelled = useRef(false), previewUrl = useRef('');
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
      if (previewUrl.current) { URL.revokeObjectURL(previewUrl.current); previewUrl.current = ''; }
    };
  }, []);

  useEffect(() => {
    if (!initial) return;
    let alive = true, revision = 0, timer: ReturnType<typeof setTimeout> | undefined;
    const poll = (): void => {
      if (!alive || ending.current) return;
      inFlight.current = (async () => {
        let limited = false;
        try {
          const next = await invoke<ScrollCaptureProgress>('capture_scroll_step');
          if (!alive || ending.current) return;
          setMessage(MESSAGES[next.status]); limited = next.status === 'limit';
          if (next.revision !== revision) {
            const bytes = await invoke<ArrayBuffer>('capture_scroll_preview');
            if (!alive || ending.current) return;
            const snapshot = decodeScrollPreview(bytes);
            const url = URL.createObjectURL(new Blob([snapshot.png], { type: 'image/png' }));
            const previous = previewUrl.current;
            previewUrl.current = url; revision = snapshot.progress.revision;
            setPreview({ url, progress: snapshot.progress });
            if (previous) URL.revokeObjectURL(previous);
          }
        } catch (captureError) {
          if (!alive || ending.current) return;
          setMessage(getErrorMessage(captureError, '无法获取页面，已保留当前长图'));
        }
        if (alive && !ending.current && !limited) timer = setTimeout(poll, 180);
      })();
    };
    poll();
    return () => { alive = false; clearTimeout(timer); };
  }, [initial]);

  const finish = async (action: CaptureOutput): Promise<void> => {
    if (!initial || cancelled.current || (ending.current && !retry.current)) return;
    ending.current = true; retry.current = false; setFinishing(true);
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
    cancelled.current = true; ending.current = true; onCancel();
  };
  cancelRef.current = cancel;

  const layout = initial?.layout;
  const progress = preview?.progress ?? initial?.progress;
  const fit = layout && progress ? Math.max(0, Math.min(
    (layout.panel.width * viewport.width / screen.width - 26) / progress.width,
    (layout.panel.height * viewport.height / screen.height - 134) / progress.height,
  )) : 0;
  return <section className="capture-scroll-overlay" aria-label="滚动截图预览" aria-busy={!initial || finishing}
    onContextMenu={(event) => { event.preventDefault(); cancel(); }}>
    {layout && <div className="capture-scroll-border" style={position(layout.selection, screen)} data-interrupted={message === MESSAGES.unmatched} />}
    <aside className="capture-scroll-panel" style={layout ? position(layout.panel, screen) : undefined} data-capture-ui>
      <header><Mouse size={22} aria-hidden="true" /><p role="status">{!initial ? '正在准备滚动截图…' : finishing ? '正在处理长图…' : message}</p><small>使用选区下方工具栏输出 · Esc 取消</small></header>
      <div className="capture-scroll-preview-area">
        {preview && <div className="capture-scroll-thumbnail" style={{ width: preview.progress.width * fit, height: preview.progress.height * fit }}>
          <img src={preview.url} alt="已截取的长图" draggable={false} />
          <div className="capture-scroll-viewport" aria-label="当前页面位置" style={{ top: `${preview.progress.viewport.y / preview.progress.height * 100}%`, height: `${preview.progress.viewport.height / preview.progress.height * 100}%` }} />
        </div>}
      </div>
      {error && <div className="capture-scroll-error" role="alert">{error}</div>}
    </aside>
    {layout && <CaptureOutputToolbar label="滚动截图操作" style={position(layout.toolbar, screen)} busy={finishing}
      onOutput={(destination) => void finish(destination)} />}
  </section>;
}
