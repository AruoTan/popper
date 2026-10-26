import { useLayoutEffect, useRef, useState, type JSX } from 'react';
import type { CaptureColorFormat, CapturePoint, CaptureRect } from '../../shared/capture';
import { clamp } from './geometry';

const SIZE = 130, ZOOM = 3, RADIUS = Math.ceil(SIZE / ZOOM / 2);

export function Magnifier({ image, point, screen, viewport, format = 'hex' }: { image: HTMLImageElement; point: CapturePoint; screen: CaptureRect; viewport: { width: number; height: number }; format?: CaptureColorFormat }): JSX.Element {
  const panel = useRef<HTMLElement>(null), canvas = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ width: SIZE, height: 226 }), [rgb, setRgb] = useState<number[] | null>(null);
  const color = rgb ? format === 'rgb' ? rgb.join(', ') : '#' + rgb.map((n) => n.toString(16).padStart(2, '0')).join('').toUpperCase() : '';
  const x = clamp(point.x, 0, screen.width - 1), y = clamp(point.y, 0, screen.height - 1);
  useLayoutEffect(() => {
    const rect = panel.current?.getBoundingClientRect();
    if (rect?.width && rect.height) setSize({ width: rect.width, height: rect.height });
  }, [viewport.width, viewport.height]);
  useLayoutEffect(() => {
    const ctx = canvas.current?.getContext('2d', { willReadFrequently: true }); if (!ctx) return;
    ctx.fillStyle = '#25292f'; ctx.fillRect(0, 0, SIZE, SIZE); ctx.imageSmoothingEnabled = false;
    const left = Math.max(0, x - RADIUS), top = Math.max(0, y - RADIUS);
    const right = Math.min(screen.width, x + RADIUS + 1), bottom = Math.min(screen.height, y + RADIUS + 1);
    ctx.drawImage(image, left, top, right - left, bottom - top,
      SIZE / 2 + (left - x - .5) * ZOOM, SIZE / 2 + (top - y - .5) * ZOOM,
      (right - left) * ZOOM, (bottom - top) * ZOOM);
    const pixel = ctx.getImageData(SIZE / 2, SIZE / 2, 1, 1).data;
    setRgb(Array.from(pixel.slice(0, 3)));
    ctx.beginPath();
    ctx.moveTo(0, SIZE / 2 + .5); ctx.lineTo(SIZE, SIZE / 2 + .5);
    ctx.moveTo(SIZE / 2 + .5, 0); ctx.lineTo(SIZE / 2 + .5, SIZE);
    ctx.lineWidth = 1; ctx.strokeStyle = '#14c36e'; ctx.stroke();
  }, [image, x, y, screen.width, screen.height]);
  const px = point.x / screen.width * viewport.width, py = point.y / screen.height * viewport.height;
  const left = px < viewport.width / 2 ? px + 24 : px - 24 - size.width;
  const top = py < viewport.height / 2 ? py + 28 : py - 28 - size.height;
  return <aside ref={panel} className="capture-magnifier" aria-label="像素放大预览" style={{ left: clamp(left, 8, Math.max(8, viewport.width - size.width - 8)), top: clamp(top, 8, Math.max(8, viewport.height - size.height - 8)) }}>
    <canvas ref={canvas} width={SIZE} height={SIZE} />
    <div className="capture-magnifier-info">
      <div><span>坐标</span><span>{screen.x + x}, {screen.y + y}</span></div>
      <div><span>色值</span><span>{color}</span></div>
      <p>按 Ctrl + C 复制色值</p>
      <p>按 Shift 切换 RGB/HEX</p>
    </div>
  </aside>;
}
