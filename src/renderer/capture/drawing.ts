import type { CaptureAnnotation, CaptureRect } from '../../shared/capture';
import { rectangle } from './geometry';

/** Draw in image pixels. The same scene is used by the overlay and PNG export. */
export function drawAnnotations(context: CanvasRenderingContext2D, items: CaptureAnnotation[]): void {
  for (const item of items) {
    const first = item.points[0], last = item.points.at(-1);
    if (!first || !last) continue;
    context.save();
    context.lineWidth = item.width;
    context.strokeStyle = item.color;
    context.fillStyle = item.color;
    context.lineCap = 'round';
    context.lineJoin = 'round';
    const rect = rectangle(first, last);
    context.beginPath();
    switch (item.tool) {
      case 'rectangle': context.strokeRect(rect.x, rect.y, rect.width, rect.height); break;
      case 'ellipse': context.ellipse(rect.x + rect.width / 2, rect.y + rect.height / 2, Math.max(.5, rect.width / 2), Math.max(.5, rect.height / 2), 0, 0, Math.PI * 2); context.stroke(); break;
      case 'arrow': {
        const angle = Math.atan2(last.y - first.y, last.x - first.x), head = Math.max(10, item.width * 4);
        context.moveTo(first.x, first.y); context.lineTo(last.x, last.y); context.stroke();
        context.beginPath(); context.moveTo(last.x, last.y);
        context.lineTo(last.x - head * Math.cos(angle - Math.PI / 6), last.y - head * Math.sin(angle - Math.PI / 6));
        context.lineTo(last.x - head * Math.cos(angle + Math.PI / 6), last.y - head * Math.sin(angle + Math.PI / 6));
        context.closePath(); context.fill(); break;
      }
      case 'pen': {
        context.moveTo(first.x, first.y);
        for (const point of item.points.slice(1)) context.lineTo(point.x, point.y);
        if (item.points.length === 1) { context.arc(first.x, first.y, item.width / 2, 0, Math.PI * 2); context.fill(); }
        else context.stroke();
        break;
      }
      case 'text': {
        const size = Math.max(14, item.width * 6);
        context.font = `${size}px system-ui, "Microsoft YaHei", sans-serif`;
        context.textBaseline = 'top';
        (item.text ?? '').split('\n').forEach((line, index) => context.fillText(line, first.x, first.y + index * size * 1.3));
        break;
      }
      case 'mosaic': {
        // Sample the composited pixels so redaction also covers earlier annotations.
        const x = Math.max(0, Math.floor(rect.x)), y = Math.max(0, Math.floor(rect.y));
        const width = Math.min(context.canvas.width - x, Math.ceil(rect.width)), height = Math.min(context.canvas.height - y, Math.ceil(rect.height));
        if (width <= 0 || height <= 0) break;
        const block = Math.max(8, item.width * 4);
        const small = document.createElement('canvas');
        small.width = Math.max(1, Math.ceil(width / block)); small.height = Math.max(1, Math.ceil(height / block));
        const sample = small.getContext('2d');
        if (!sample) throw new Error('无法创建马赛克画布');
        sample.drawImage(context.canvas, x, y, width, height, 0, 0, small.width, small.height);
        context.imageSmoothingEnabled = false;
        context.drawImage(small, 0, 0, small.width, small.height, x, y, width, height);
        break;
      }
    }
    context.restore();
  }
}
export function renderScene(canvas: HTMLCanvasElement, image: CanvasImageSource, width: number, height: number, items: CaptureAnnotation[]): void {
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (!context) throw new Error('无法创建图像画布');
  context.clearRect(0, 0, width, height);
  context.drawImage(image, 0, 0, width, height);
  drawAnnotations(context, items);
}
export function translateAnnotations(items: CaptureAnnotation[], rect: CaptureRect): CaptureAnnotation[] {
  return items.map((item) => ({ ...item, points: item.points.map((point) => ({ x: point.x - rect.x, y: point.y - rect.y })) }));
}
export async function pngBytes(canvas: HTMLCanvasElement): Promise<Uint8Array> {
  const blob = await new Promise<Blob>((resolve, reject) => canvas.toBlob((value) => value ? resolve(value) : reject(new Error('无法导出图片')), 'image/png'));
  return new Uint8Array(await blob.arrayBuffer());
}
export async function decodePng(bytes: ArrayBuffer | number[]): Promise<HTMLImageElement> {
  const url = URL.createObjectURL(new Blob([bytes instanceof ArrayBuffer ? bytes : new Uint8Array(bytes)], { type: 'image/png' }));
  try {
    const image = new Image(); image.src = url; await image.decode(); return image;
  } finally { URL.revokeObjectURL(url); }
}
