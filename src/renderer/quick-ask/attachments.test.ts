import { describe, expect, it } from 'vitest';
import { readAttachment, validateAttachmentList } from './attachments';
import { captureQuickAskShortcut } from './shortcut';

describe('Quick Ask attachments and shortcuts', () => {
  it('reads PDF bytes and UTF-8 text rather than a local path', async () => {
    const pdf = await readAttachment(new File(['%PDF-1.7'], 'paper.pdf'));
    expect(pdf.payload).toEqual({ kind: 'file', name: 'paper.pdf', mime: 'application/pdf', data: btoa('%PDF-1.7') });
    const text = await readAttachment(new File(['附件内容'], 'note.txt'));
    expect(text.payload.data).toBe('附件内容');
  });
  it('preserves Word and other binary files for DSH upload', async () => {
    const mime = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';
    const word = await readAttachment(new File(['PK\x03\x04example'], 'report.docx', { type: mime }));
    expect(word.payload).toEqual({ name: 'report.docx', kind: 'file', mime, data: btoa('PK\x03\x04example') });
    const binary = await readAttachment(new File([new Uint8Array([0, 1, 255])], 'data.bin'));
    expect(binary.payload).toMatchObject({ kind: 'file', mime: 'application/octet-stream' });
  });

  it('bounds size and text length and rejects binary disguised as text', async () => {
    await expect(readAttachment(new File([''], 'empty.txt'))).rejects.toThrow('空文件');
    await expect(readAttachment(new File(['a\0b'], 'binary.txt'))).rejects.toThrow('UTF-8');
    const file = await readAttachment(new File(['a'], 'a.txt'));
    expect(() => validateAttachmentList(Array(7).fill(file))).not.toThrow();
    expect(() => validateAttachmentList([{ ...file, size: 21 * 1024 * 1024 }])).toThrow('20 MB');
    expect(() => validateAttachmentList([{ ...file, payload: { ...file.payload, data: 'a'.repeat(60_001) } }])).toThrow('60,000');
  });
  it('captures modified Space and physical key combinations', () => {
    const keys = { code: 'Space', ctrlKey: true, altKey: false, shiftKey: true, metaKey: false };
    expect(captureQuickAskShortcut(keys)).toBe('Control+Shift+Space');
    expect(captureQuickAskShortcut({ ...keys, code: 'KeyK', shiftKey: false })).toBe('Control+KeyK');
    expect(captureQuickAskShortcut({ ...keys, code: 'ShiftLeft' })).toBeNull();
    expect(captureQuickAskShortcut({ ...keys, ctrlKey: false, shiftKey: false })).toBeNull();
  });
});
