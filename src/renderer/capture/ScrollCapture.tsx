import { useEffect, useRef, useState, type JSX } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Check, Pause, Play, X } from 'lucide-react';
import type { ScrollCaptureProgress } from '../../shared/capture';
import { getErrorMessage } from '../lib/errors';

const MESSAGES: Record<ScrollCaptureProgress['status'], string> = {
  appended: '已扩展长图，可继续向上或向下滚动',
  tracked: '已定位到捕捉过的区域，可继续上下滚动',
  unchanged: '在选区内上下滚动，或开启自动向下滚动',
  unmatched: '出现未匹配区域，请滚回任一已捕捉区域后继续',
  limit: '已达到长图容量上限，请点击完成',
};

export function ScrollCapture({ initial, onFinish, onCancel }: {
  initial: ScrollCaptureProgress | null;
  onFinish: () => Promise<boolean>;
  onCancel: () => void;
}): JSX.Element {
  const [progress, setProgress] = useState(initial);
  const [automatic, setAutomatic] = useState(false), [finishing, setFinishing] = useState(false), [retry, setRetry] = useState(false);
  const [message, setMessage] = useState(MESSAGES.unchanged);
  const auto = useRef(false), ending = useRef(false), unchanged = useRef(0), inFlight = useRef<Promise<void>>(Promise.resolve());
  const mounted = useRef(true), cancelled = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    if (!initial) return;
    setProgress(initial);
    let alive = true, timer: ReturnType<typeof setTimeout> | undefined;
    const pause = (): void => { auto.current = false; setAutomatic(false); };
    const poll = (): void => {
      if (!alive || ending.current) return;
      inFlight.current = (async () => {
        try {
          const wasAutomatic = auto.current;
          const next = await invoke<ScrollCaptureProgress>('capture_scroll_step', { automatic: wasAutomatic });
          if (!alive || ending.current) return;
          setProgress(next); setMessage(MESSAGES[next.status]);
          if (next.status !== 'appended' && next.status !== 'tracked' && next.status !== 'unchanged') pause();
          if (next.status === 'limit') return;
          unchanged.current = wasAutomatic && next.status === 'unchanged' ? unchanged.current + 1 : 0;
          if (unchanged.current >= 3) { pause(); setMessage('画面未继续滚动，已暂停；可手动滚动或点击完成'); }
        } catch (error) {
          if (!alive || ending.current) return;
          pause(); setMessage(getErrorMessage(error, '无法获取页面，已保留当前长图'));
        }
        if (alive && !ending.current) timer = setTimeout(poll, 180);
      })();
    };
    timer = setTimeout(poll, 180);
    return () => { alive = false; clearTimeout(timer); };
  }, [initial]);

  const finish = async (): Promise<void> => {
    if (!initial || (ending.current && !retry)) return;
    ending.current = true; auto.current = false; setAutomatic(false); setFinishing(true); setRetry(false);
    await inFlight.current;
    if (!mounted.current || cancelled.current) return;
    const finished = await onFinish();
    if (!finished && mounted.current && !cancelled.current) { setFinishing(false); setRetry(true); setMessage('无法打开长图，内容已保留，请重试完成'); }
  };
  return <section className="capture-scroll-controller" data-capture-ui aria-label="滚动截图控制栏">
    <header onPointerDown={(event) => { if (event.button === 0) void getCurrentWindow().startDragging().catch(() => {}); }}><strong>滚动截图</strong><span>{progress ? `${progress.width} × ${progress.height} · ${progress.frames} 帧` : '正在准备…'}</span></header>
    <p role="status">{finishing ? '正在生成长图…' : message}</p>
    <div>
      <button disabled={!initial || ending.current || progress?.status === 'limit'} aria-pressed={automatic} onClick={() => { auto.current = !auto.current; unchanged.current = 0; setAutomatic(auto.current); }}>{automatic ? <Pause size={16} /> : <Play size={16} />}{automatic ? '暂停自动滚动' : '自动向下滚动'}</button>
      <button disabled={!initial || finishing} onClick={() => void finish()}><Check size={16} />{retry ? '重试完成' : '完成'}</button>
      <button onClick={() => { cancelled.current = true; ending.current = true; auto.current = false; setAutomatic(false); onCancel(); }}><X size={16} />取消</button>
    </div>
    <small>手动滚动时将鼠标放在选区内 · Esc 取消</small>
  </section>;
}
