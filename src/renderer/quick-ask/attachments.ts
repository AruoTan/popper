import type { QuickAskAttachment } from '../../shared';

export const ATTACHMENT_ACCEPT = "";
export const MAX_ATTACHMENTS = 6;
const MAX_BYTES = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES = 20 * 1024 * 1024;
const IMAGE_TYPES: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp' };
const TEXT_EXTENSIONS = new Set(['txt', 'md', 'csv', 'json', 'log', 'xml', 'yaml', 'yml', 'html', 'css', 'js', 'ts', 'tsx', 'jsx', 'py', 'rs', 'toml']);

export interface SelectedAttachment { id: string; size: number; payload: QuickAskAttachment }

function readFile(file: File, binary: boolean): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error(`无法读取 ${file.name}`));
    reader.onload = () => {
      if (typeof reader.result !== 'string') return reject(new Error(`无法读取 ${file.name}`));
      resolve(binary ? reader.result.slice(reader.result.indexOf(',') + 1) : reader.result);
    };
    if (binary) reader.readAsDataURL(file); else reader.readAsText(file, 'UTF-8');
  });
}

export async function readAttachment(file: File): Promise<SelectedAttachment> {
  if (!file.size) throw new Error(`${file.name} 是空文件`);
  if (file.size > MAX_BYTES) throw new Error(`${file.name} 超过 8 MB`);
  if (file.name.length > 255 || /[\u0000-\u001f\u007f]/.test(file.name)) throw new Error('附件名称无效');
  const ext = file.name.split('.').at(-1)?.toLowerCase() ?? '';
  const imageMime = IMAGE_TYPES[ext] ?? (Object.values(IMAGE_TYPES).includes(file.type) ? file.type : undefined);
  const kind = imageMime ? 'image' : TEXT_EXTENSIONS.has(ext) || file.type.startsWith('text/') ? 'text' : 'file';
  const data = await readFile(file, kind !== 'text');
  if (kind === 'text' && (data.includes('\0') || data.includes('\ufffd'))) throw new Error(`${file.name} 不是有效的 UTF-8 文本文件`);
  return { id: crypto.randomUUID(), size: file.size, payload: { name: file.name, mime: imageMime ?? (kind === 'text' ? 'text/plain' : ext === 'pdf' ? 'application/pdf' : file.type || 'application/octet-stream'), kind, data } };
}

export function validateAttachmentList(attachments: SelectedAttachment[]): void {
  if (attachments.length > MAX_ATTACHMENTS) throw new Error('最多添加 6 个附件');
  if (attachments.reduce((size, file) => size + file.size, 0) > MAX_TOTAL_BYTES) throw new Error('附件总大小不能超过 20 MB');
  if (attachments.reduce((length, file) => length + (file.payload.kind === 'text' ? Array.from(file.payload.data).length : 0), 0) > 60_000) throw new Error('文本附件合计不能超过 60,000 个字符');
}

export function attachmentSize(bytes: number): string {
  return bytes < 1024 * 1024 ? `${Math.max(1, Math.round(bytes / 1024))} KB` : `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
