import type { ScrollCaptureProgress } from '../../shared/capture';

export function decodeScrollPreview(bytes: ArrayBuffer): { progress: ScrollCaptureProgress; png: ArrayBuffer } {
  if (bytes.byteLength < 4) throw new Error('长图预览数据不完整');
  const length = new DataView(bytes).getUint32(0, true);
  if (length > 4096 || length + 12 > bytes.byteLength) throw new Error('长图预览数据不完整');
  const progress = JSON.parse(new TextDecoder().decode(new Uint8Array(bytes, 4, length))) as ScrollCaptureProgress;
  if (!Number.isSafeInteger(progress.revision) || progress.revision < 1 || !(progress.width > 0 && progress.height > 0)
    || !progress.viewport || progress.viewport.y < 0 || progress.viewport.height <= 0
    || progress.viewport.y + progress.viewport.height > progress.height) throw new Error('长图预览尺寸无效');
  return { progress, png: bytes.slice(4 + length) };
}
