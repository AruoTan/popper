import { useEffect, useLayoutEffect, useRef, type JSX } from 'react';
import type { PreparedScrollPreview } from './scrollPreview';

const MOTION_MS = 80;

export function ScrollCapturePreview({ preview, width, height }: {
  preview: PreparedScrollPreview;
  width: number;
  height: number;
}): JSX.Element {
  const thumbnail = useRef<HTMLDivElement>(null);
  const motion = useRef<{ from: number; target: number; animation: Animation | null } | null>(null);
  const { progress } = preview;
  const fit = Math.max(0, Math.min(width / progress.width, height / progress.height));
  useLayoutEffect(() => {
    const element = thumbnail.current; if (!element) return;
    const previous = motion.current;
    let current = previous?.target ?? fit;
    if (previous?.animation && typeof previous.animation.currentTime === 'number') {
      const elapsed = Math.min(1, Math.max(0, previous.animation.currentTime / MOTION_MS));
      current = previous.from + (previous.target - previous.from) * elapsed;
    }
    previous?.animation?.cancel();
    const next = { from: current, target: fit, animation: null as Animation | null };
    motion.current = next;
    if (fit > 0 && Math.abs(current - fit) > 0.000001 && element.animate
      && !window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
      // Animate one uniform scale so newly appended rows cannot stretch the image.
      next.animation = element.animate([
        { transform: `scale(${current / fit})` }, { transform: 'scale(1)' },
      ], { duration: MOTION_MS, easing: 'linear' });
    }
  }, [fit]);
  useEffect(() => {
    const media = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    const changed = (): void => { if (media?.matches) motion.current?.animation?.cancel(); };
    media?.addEventListener?.('change', changed);
    return () => { media?.removeEventListener?.('change', changed); motion.current?.animation?.cancel(); };
  }, []);
  return <div className="capture-scroll-preview-content" style={{ width, height }}>
    <div ref={thumbnail} className="capture-scroll-thumbnail" style={{ width: progress.width * fit, height: progress.height * fit }}>
      <img src={preview.url} alt="已截取的长图" draggable={false} />
    </div>
    <div className="capture-scroll-viewport" aria-label="当前页面位置" style={{ left: (width - progress.width * fit) / 2, top: progress.viewport.y * fit, width: progress.width * fit, height: progress.viewport.height * fit }} />
  </div>;
}
