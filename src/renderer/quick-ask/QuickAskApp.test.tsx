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
