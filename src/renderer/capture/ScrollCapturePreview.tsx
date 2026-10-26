import { useEffect, useLayoutEffect, useRef, type JSX } from 'react';
import type { PreparedScrollPreview } from './scrollPreview';

const MOTION_MS = 160;
interface Geometry { scale: number; height: number; offset: number; markerY: number; markerHeight: number }
const mix = (from: number, to: number, amount: number): number => from + (to - from) * amount;

export function ScrollCapturePreview({ preview, width, height, align = 'left' }: {
  preview: PreparedScrollPreview;
  width: number;
  height: number;
  align?: 'left' | 'right';
}): JSX.Element {
  const thumbnail = useRef<HTMLDivElement>(null), image = useRef<HTMLImageElement>(null), marker = useRef<HTMLDivElement>(null);
  const motion = useRef<{ current: Geometry; target: Geometry; origin: number; frame: number } | null>(null);
  const { progress, originY } = preview;
  const fit = Math.max(0, Math.min(width / progress.width, height / progress.height));
  const paint = useRef<(geometry: Geometry) => void>(() => {});
  paint.current = (geometry): void => {
    const box = thumbnail.current, img = image.current, viewport = marker.current;
    if (!box || !img || !viewport) return;
    const imageWidth = progress.width * geometry.scale;
    const left = align === 'right' ? width - imageWidth : 0;
    box.style.left = `${left}px`;
    box.style.width = `${imageWidth}px`; box.style.height = `${geometry.height}px`;
    img.style.transform = `translateY(${geometry.offset}px) scale(${geometry.scale})`;
    viewport.style.left = `${left}px`; viewport.style.width = `${imageWidth}px`;
    viewport.style.top = `${height - geometry.height + geometry.markerY}px`; viewport.style.height = `${geometry.markerHeight}px`;
  };
  useLayoutEffect(() => {
    const target: Geometry = { scale: fit, height: progress.height * fit, offset: 0, markerY: progress.viewport.y * fit, markerHeight: progress.viewport.height * fit };
    const previous = motion.current;
    if (previous) cancelAnimationFrame(previous.frame);
    const from = previous ? { ...previous.current, offset: previous.current.offset + (originY - previous.origin) * previous.current.scale } : target;
    const state = { current: from, target, origin: originY, frame: 0 };
    motion.current = state;
    const reduced = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    if (!previous || reduced || Object.keys(target).every((key) => from[key as keyof Geometry] === target[key as keyof Geometry])) {
      state.current = target; paint.current(target); return;
    }
    // Reveal new rows and retarget the displayed geometry without stretching
    // pixels or jumping when content arrives above the starting view.
    paint.current(from);
    const start = performance.now();
    const tick = (now: number): void => {
      const elapsed = Math.min(1, Math.max(0, (now - start) / MOTION_MS));
      const amount = 1 - (1 - elapsed) ** 3;
      state.current = {
        scale: mix(from.scale, target.scale, amount), height: mix(from.height, target.height, amount),
        offset: mix(from.offset, target.offset, amount), markerY: mix(from.markerY, target.markerY, amount),
        markerHeight: mix(from.markerHeight, target.markerHeight, amount),
      };
      // A large update can cross the screen top in one step. Grow to that
      // boundary before shrinking, rather than scaling while space remains.
      if (from.scale > target.scale && from.height < height && target.height >= height && from.scale > 0) {
        const revealed = mix(from.height / from.scale, progress.height, amount);
        const scale = Math.min(from.scale, height / revealed);
        state.current = {
          scale, height: revealed * scale,
          offset: mix(from.offset / from.scale, 0, amount) * scale,
          markerY: mix(from.markerY / from.scale, progress.viewport.y, amount) * scale,
          markerHeight: mix(from.markerHeight / from.scale, progress.viewport.height, amount) * scale,
        };
      }
      paint.current(state.current);
      state.frame = elapsed < 1 ? requestAnimationFrame(tick) : 0;
    };
    state.frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(state.frame);
  }, [fit, width, height, align, progress.width, progress.height, progress.viewport.y, progress.viewport.height, originY]);
  useEffect(() => {
    const media = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    const changed = (): void => {
      const state = motion.current;
      if (media?.matches && state) { cancelAnimationFrame(state.frame); state.current = state.target; paint.current(state.target); }
    };
    media?.addEventListener?.('change', changed);
    return () => { media?.removeEventListener?.('change', changed); if (motion.current) cancelAnimationFrame(motion.current.frame); };
  }, []);
  return <div className="capture-scroll-preview-content" style={{ width, height }}>
    <div ref={thumbnail} className="capture-scroll-thumbnail">
      <img ref={image} src={preview.url} alt="已截取的长图" draggable={false} style={{ width: progress.width, height: progress.height }} />
    </div>
    <div ref={marker} className="capture-scroll-viewport" aria-label="当前页面位置" />
  </div>;
}
