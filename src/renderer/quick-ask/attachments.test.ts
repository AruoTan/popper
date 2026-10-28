import { describe, expect, it, vi } from 'vitest';
import { appendAttachments, readAttachment, validateAttachmentList } from './attachments';
import { captureQuickAskShortcut } from './shortcut';

describe('Quick Ask attachments and shortcuts', () => {
  it('rejects an oversized batch before reading any file, including existing attachments', async () => {
    const current = await readAttachment(new File(['context'], 'context.txt'));
    const readBinary = vi.spyOn(FileReader.prototype, 'readAsDataURL');
    const readText = vi.spyOn(FileReader.prototype, 'readAsText');
    const file = new File([new Uint8Array(7 * 1024 * 1024)], 'large.bin');
    await expect(appendAttachments([], [file, file, file])).rejects.toThrow('20 MB');
    await expect(appendAttachments([{ ...current, size: 14 * 1024 * 1024 }], [file])).rejects.toThrow('20 MB');
    expect(readBinary).not.toHaveBeenCalled();
    expect(readText).not.toHaveBeenCalled();
    expect(current.payload.data).toBe('context');
  });

  it.each(['empty', 'oversized', 'invalid name'])('validates every file before reading a batch containing an %s file', async (kind) => {
    const valid = new File(['valid'], 'first.txt');
    const invalid = kind === 'empty' ? new File([], 'empty.txt')
      : kind === 'oversized' ? new File([new Uint8Array(8 * 1024 * 1024 + 1)], 'large.bin')
      : new File(['data'], 'invalid\u0001.txt');
    const readBinary = vi.spyOn(FileReader.prototype, 'readAsDataURL');
    const readText = vi.spyOn(FileReader.prototype, 'readAsText');
    await expect(appendAttachments([], [valid, invalid])).rejects.toThrow();
    expect(readBinary).not.toHaveBeenCalled();
    expect(readText).not.toHaveBeenCalled();
  });

  it('appends in selection order and leaves the existing list unchanged on text validation failure', async () => {
    const current = [await readAttachment(new File(['保留😀'], 'context.txt'))];
    const next = await appendAttachments(current, [new File(['%PDF-1.7'], 'paper.pdf'), new File(['中文😀'], 'note.txt')]);
    expect(next.map(({ payload }) => payload.name)).toEqual(['context.txt', 'paper.pdf', 'note.txt']);
    expect(current).toHaveLength(1);
    await expect(appendAttachments(current, [new File(['字'.repeat(60_000)], 'long.txt')])).rejects.toThrow('60,000');
    await expect(appendAttachments(current, [new File(['bad\0data'], 'bad.txt')])).rejects.toThrow('UTF-8');
    expect(current[0]?.payload.data).toBe('保留😀');
    expect(current).toHaveLength(1);
  });

  it('counts Chinese characters and supplementary Unicode scalars at the exact text limit', async () => {
    const item = await readAttachment(new File(['中😀𠮷'.repeat(20_000)], 'unicode.txt'));
    expect(() => validateAttachmentList([item])).not.toThrow();
    expect(() => validateAttachmentList([{ ...item, payload: { ...item.payload, data: item.payload.data + '😀' } }])).toThrow('60,000');
  });

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
    expect(captureQuickAskShortcut({ ...keys, ctrlKey: false, metaKey: true, code: 'KeyS' })).toBe('Super+Shift+KeyS');
    expect(captureQuickAskShortcut({ ...keys, ctrlKey: false, shiftKey: false, code: 'PrintScreen' })).toBe('PrintScreen');
    expect(captureQuickAskShortcut({ ...keys, ctrlKey: false, shiftKey: false, code: 'F12' })).toBe('F12');
    expect(captureQuickAskShortcut({ ...keys, code: 'ShiftLeft' })).toBeNull();
    expect(captureQuickAskShortcut({ ...keys, ctrlKey: false, shiftKey: false })).toBeNull();
  });
});
