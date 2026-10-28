import type { ScrollCaptureProgress } from '../../shared/capture';

export function decodeScrollPreview(bytes: ArrayBuffer): { progress: ScrollCaptureProgress; originY: number; png: ArrayBuffer } {
  if (bytes.byteLength < 4) throw new Error('长图预览数据不完整');
  const length = new DataView(bytes).getUint32(0, true);
  if (length > 4096 || length + 12 > bytes.byteLength) throw new Error('长图预览数据不完整');
  const progress = JSON.parse(new TextDecoder().decode(new Uint8Array(bytes, 4, length))) as ScrollCaptureProgress & { originY?: number };
  const originY = progress.originY ?? 0;
  if (!Number.isSafeInteger(progress.revision) || progress.revision < 1 || !(progress.width > 0 && progress.height > 0)
    || !progress.viewport || progress.viewport.y < 0 || progress.viewport.height <= 0
    || progress.viewport.y + progress.viewport.height > progress.height || !Number.isSafeInteger(originY)) throw new Error('长图预览尺寸无效');
  return { progress, originY, png: bytes.slice(4 + length) };
}

export interface PreparedScrollPreview { url: string; progress: ScrollCaptureProgress; originY: number }

/** Keep the previous thumbnail visible until both its replacement and metadata are ready. */
export async function prepareScrollPreview(bytes: ArrayBuffer, signal: AbortSignal): Promise<PreparedScrollPreview> {
  const snapshot = decodeScrollPreview(bytes);
  if (signal.aborted) throw new DOMException('预览已取消', 'AbortError');
  const url = URL.createObjectURL(new Blob([snapshot.png], { type: 'image/png' }));
  const image = new Image(); image.src = url;
  try {
    await new Promise<void>((resolve, reject) => {
      const abort = (): void => { image.src = ''; reject(new DOMException('预览已取消', 'AbortError')); };
      signal.addEventListener('abort', abort, { once: true });
      void image.decode().then(resolve, reject).finally(() => signal.removeEventListener('abort', abort));
    });
    return { url, progress: snapshot.progress, originY: snapshot.originY };
  } catch (error) {
    image.src = ''; URL.revokeObjectURL(url); throw error;
  }
}
