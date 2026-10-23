import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { WindowPopperApi } from '../../shared';
import { QuickAskApp } from './QuickAskApp';

const submit = vi.fn();
const close = vi.fn();
let opened: (() => void) | undefined;
beforeEach(() => {
  submit.mockReset().mockResolvedValue({ accepted: true, sessionId: 'session' });
  close.mockReset().mockResolvedValue(undefined);
  Object.defineProperty(window, '_popper_', { configurable: true, value: {
    submitQuickAsk: submit, closeQuickAsk: close,
    quickAskReady: vi.fn().mockResolvedValue(undefined),
    onQuickAskOpened: (listener: () => void) => { opened = listener; return () => { opened = undefined; }; },
  } as unknown as WindowPopperApi });
});

describe('Quick Ask', () => {
  it('starts with no preview and sends a question without selection', async () => {
    render(<QuickAskApp />);
    expect(screen.queryByRole('region', { name: '附件预览' })).toBeNull();
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '  Explain Docker  ' } });
    fireEvent.click(screen.getByRole('button', { name: '发送问题' }));
    await waitFor(() => expect(submit).toHaveBeenCalledWith('Explain Docker', []));
    await waitFor(() => expect(screen.getByRole('textbox')).toHaveValue(''));
  });

  it.each(['click', 'Enter'])('sends attachments without text via %s and still rejects empty input', async (method) => {
    render(<QuickAskApp />);
    const input = screen.getByRole('textbox');
    const send = screen.getByRole('button', { name: '发送问题' });
    fireEvent.change(input, { target: { value: '   ' } });
    expect(send).toBeDisabled();
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(submit).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText('选择附件'), {
      target: { files: [new File(['file contents'], 'note.txt', { type: 'text/plain' })] },
    });
    await waitFor(() => expect(send).toBeEnabled());
    if (method === 'click') fireEvent.click(send);
    else fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith('', [
      expect.objectContaining({ name: 'note.txt', kind: 'text', data: 'file contents' }),
    ]));
    await waitFor(() => expect(screen.queryByRole('region', { name: '附件预览' })).toBeNull());
    expect(input).toHaveValue('');
    expect(send).toBeDisabled();
  });

  it('previews actual file contents and removes the region with the last attachment', async () => {
    render(<QuickAskApp />);
    const file = new File(['hello world'], 'note.md', { type: 'text/plain' });
    fireEvent.change(screen.getByLabelText('选择附件'), { target: { files: [file] } });
    await screen.findByText('note.md');
    expect(screen.getByRole('region', { name: '附件预览' })).toBeInTheDocument();
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'summarize' } });
    submit.mockResolvedValueOnce({ accepted: false, message: 'try again' });
    fireEvent.click(screen.getByRole('button', { name: '发送问题' }));
    await screen.findByRole('alert');
    expect(submit).toHaveBeenCalledWith('summarize', [expect.objectContaining({ name: 'note.md', kind: 'text', data: 'hello world' })]);
    expect(screen.getByRole('textbox')).toHaveValue('summarize');
    fireEvent.click(screen.getByRole('button', { name: '移除 note.md' }));
    expect(screen.queryByRole('region', { name: '附件预览' })).toBeNull();
  });

  it('accepts pasted images and displays a thumbnail', async () => {
    render(<QuickAskApp />);
    const image = new File([new Uint8Array([137,80,78,71,13,10,26,10])], 'image.png', { type: 'image/png' });
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: { files: [image] } });
    const thumbnail = await screen.findByRole('img', { name: 'image.png' });
    expect(thumbnail).toHaveAttribute('src', expect.stringContaining('data:image/png;base64,'));
  });

  it('adds more than six attachments across batches and displays submission errors', async () => {
    render(<QuickAskApp />);
    const files = Array.from({ length: 7 }, (_, index) => new File(['content'], `note-${index}.txt`));
    const picker = screen.getByLabelText('选择附件');
    fireEvent.change(picker, { target: { files: files.slice(0, 6) } });
    await screen.findByText('6 个附件');
    fireEvent.change(picker, { target: { files: files.slice(6) } });
    await screen.findByText('7 个附件');
    expect(screen.getAllByRole('button', { name: /^移除 note-/ })).toHaveLength(7);
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'summarize' } });
    submit.mockResolvedValueOnce({ accepted: false, message: '服务商拒绝：附件数量超过限制' });
    fireEvent.click(screen.getByRole('button', { name: '发送问题' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('附件数量超过限制');
    expect(submit).toHaveBeenCalledWith('summarize', expect.arrayContaining([
      expect.objectContaining({ name: 'note-0.txt' }),
      expect.objectContaining({ name: 'note-6.txt' }),
    ]));
    expect(submit.mock.calls[0]?.[1]).toHaveLength(7);
    expect(screen.getByRole('textbox')).toHaveValue('summarize');
    expect(screen.getByText('7 个附件')).toBeInTheDocument();
  });

  it('rejects invalid text without adding an empty preview', async () => {
    render(<QuickAskApp />);
    fireEvent.drop(screen.getByRole('main'), { dataTransfer: { files: [new File(['a\0b'], 'binary.txt')] } });
    expect(await screen.findByRole('alert')).toHaveTextContent('UTF-8');
    expect(screen.queryByRole('region', { name: '附件预览' })).toBeNull();
  });

  it('preserves draft on reopen and supports Escape, IME and Shift+Enter', async () => {
    render(<QuickAskApp />);
    const input = screen.getByRole('textbox');
    fireEvent.change(input, { target: { value: 'draft' } });
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true });
    fireEvent.keyDown(input, { key: 'Enter', shiftKey: true });
    expect(submit).not.toHaveBeenCalled();
    act(() => opened?.());
    expect(input).toHaveValue('draft');
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(close).toHaveBeenCalledOnce();
  });

  it('coalesces repeated submits while the native reveal is pending', async () => {
    let resolve!: (value: { accepted: true }) => void;
    submit.mockImplementation(() => new Promise((done) => { resolve = done; }));
    render(<QuickAskApp />);
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'test' } });
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
    expect(submit).toHaveBeenCalledOnce();
    await act(async () => resolve({ accepted: true }));
  });
});

it('receives a captured image once while preserving the existing question draft', async () => {
  let available: { id: string; attachment: { name: string; mime: string; kind: 'image'; data: string } } | null = null;
  let receive: (() => void) | undefined;
  const acknowledge = vi.fn(async () => { available = null; });
  window._popper_.pendingCaptureAttachment = vi.fn(async () => available);
  window._popper_.ackCaptureAttachment = acknowledge;
  window._popper_.onCaptureAttachment = (listener) => { receive = listener; return () => { receive = undefined; }; };
  render(<QuickAskApp />);
  fireEvent.change(screen.getByRole('textbox'), { target: { value: '保留这段草稿' } });
  await act(async () => {});
  available = { id: 'capture-1', attachment: { name: 'Popper-capture.png', mime: 'image/png', kind: 'image' as const, data: 'iVBORw0KGgo=' } };
  act(() => { receive?.(); receive?.(); });
  await screen.findByRole('img', { name: 'Popper-capture.png' });
  expect(acknowledge).toHaveBeenCalledExactlyOnceWith('capture-1');
  expect(screen.getByRole('textbox')).toHaveValue('保留这段草稿');
  expect(submit).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '发送问题' }));
  await waitFor(() => expect(submit).toHaveBeenCalledWith('保留这段草稿', [expect.objectContaining({ name: 'Popper-capture.png' })]));
});

it('rolls back a captured attachment if its native acknowledgement expires', async () => {
  window._popper_.pendingCaptureAttachment = vi.fn(async () => ({ id: 'expired', attachment: { name: 'expired.png', mime: 'image/png', kind: 'image' as const, data: 'iVBORw0KGgo=' } }));
  window._popper_.ackCaptureAttachment = vi.fn().mockRejectedValue(new Error('图片交接已结束'));
  render(<QuickAskApp />);
  expect(await screen.findByRole('alert')).toHaveTextContent('图片交接已结束');
  expect(screen.queryByRole('region', { name: '附件预览' })).toBeNull();
  expect(submit).not.toHaveBeenCalled();
});

it('keeps capture integration inactive on a platform without screen capture', async () => {
  window._popper_.getAccessibilityStatus = vi.fn(async () => ({ platform: 'darwin' as const, trusted: true, canRequest: false, available: true, diagnostics: {} }));
  window._popper_.pendingCaptureAttachment = vi.fn().mockRejectedValue(new Error('unsupported'));
  window._popper_.startCapture = vi.fn();
  render(<QuickAskApp />);
  await act(async () => {});
  expect(window._popper_.pendingCaptureAttachment).not.toHaveBeenCalled();
  expect(screen.queryByRole('button', { name: '截图并添加附件' })).toBeNull();
  expect(screen.queryByRole('alert')).toBeNull();
});
