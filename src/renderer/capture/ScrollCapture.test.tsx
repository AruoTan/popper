import { act, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ScrollCaptureProgress, ScrollCaptureSession } from '../../shared/capture';
import { ScrollCapture } from './ScrollCapture';
import { decodeScrollPreview } from './scrollPreview';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
const screen = { x: -1920, y: -1080, width: 1920, height: 1080 };
const progress: ScrollCaptureProgress = { width: 800, height: 600, revision: 1, viewport: { x: 0, y: 0, width: 800, height: 600 }, status: 'unchanged' };
const initial: ScrollCaptureSession = { progress, layout: { selection: { x: 100, y: 100, width: 800, height: 600 }, panel: { x: 924, y: 100, width: 324, height: 765 } } };
function packet(state = progress): ArrayBuffer {
  const metadata = new TextEncoder().encode(JSON.stringify(state));
  const bytes = new Uint8Array(4 + metadata.length + 8);
  new DataView(bytes.buffer).setUint32(0, metadata.length, true);
  bytes.set(metadata, 4); bytes.set([137, 80, 78, 71, 13, 10, 26, 10], 4 + metadata.length);
  return bytes.buffer;
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(invoke).mockReset().mockImplementation(async (command) => command === 'capture_scroll_preview' ? packet() : progress);
  vi.mocked(listen).mockReset().mockResolvedValue(vi.fn());
  vi.stubGlobal('URL', { createObjectURL: vi.fn().mockReturnValue('blob:preview'), revokeObjectURL: vi.fn() });
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });
const advance = async (ms = 180): Promise<void> => { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); };
const overlay = (view: ReturnType<typeof render>): HTMLElement => view.getByLabelText('滚动截图预览');
const flush = async (): Promise<void> => { await act(async () => {}); };

describe('manual scrolling overlay', () => {
  it('waits for initialization and serializes capture requests without a control button row or frame count', async () => {
    const view = render(<ScrollCapture initial={null} screen={screen} onFinish={vi.fn()} onCancel={vi.fn()} />);
    await advance(1000); expect(invoke).not.toHaveBeenCalled();
    expect(overlay(view)).toHaveAttribute('aria-busy', 'true');
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    view.rerender(<ScrollCapture initial={initial} screen={screen} onFinish={vi.fn()} onCancel={vi.fn()} />);
    await advance(2000); expect(invoke).toHaveBeenCalledTimes(1);
    await act(async () => { resolve(progress); });
    expect(invoke).toHaveBeenCalledWith('capture_scroll_step');
    expect(invoke).toHaveBeenCalledWith('capture_scroll_preview');
    expect(view.queryByRole('button')).toBeNull(); expect(view.queryByText(/帧/)).toBeNull(); view.unmount();
  });
  it('shows a correctly placed selection and synchronizes preview dimensions and viewport from one snapshot', async () => {
    const snapshot = { ...progress, height: 2400, revision: 4, viewport: { ...progress.viewport, y: 1200 } };
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_scroll_preview' ? packet(snapshot) : progress);
    const view = render(<ScrollCapture initial={initial} screen={screen} onFinish={vi.fn()} onCancel={vi.fn()} />); await flush();
    expect(view.getByRole('img', { name: '已截取的长图' })).toHaveAttribute('src', 'blob:preview');
    const marker = view.getByLabelText('当前页面位置');
    expect(marker.style.top).toBe('50%'); expect(marker.style.height).toBe('25%');
    const border = view.container.querySelector<HTMLElement>('.capture-scroll-border')!;
    expect(parseFloat(border.style.left)).toBeCloseTo(100 / 1920 * 100);
    expect(parseFloat(border.style.top)).toBeCloseTo(100 / 1080 * 100);
    view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:preview');
  });
  it('refreshes previews only when the document changes and releases replaced images', async () => {
    const view = render(<ScrollCapture initial={initial} screen={screen} onFinish={vi.fn()} onCancel={vi.fn()} />);
    await advance(540);
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'capture_scroll_preview')).toHaveLength(1);
    const next = { ...progress, revision: 2, status: 'tracked' as const };
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_scroll_preview' ? packet(next) : next);
    await advance(); expect(URL.createObjectURL).toHaveBeenCalledTimes(2); expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
    view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
  });
  it.each(['unmatched', 'limit'] as const)('preserves the preview and gives a passive hint on %s', async (status) => {
    const view = render(<ScrollCapture initial={initial} screen={screen} onFinish={vi.fn()} onCancel={vi.fn()} />); await flush();
    vi.mocked(invoke).mockResolvedValue({ ...progress, status }); await advance();
    expect(view.getByRole('status')).toHaveTextContent(status === 'limit' ? '容量上限' : '滚回已截取的位置');
    expect(view.getByRole('img')).toBeInTheDocument();
    if (status === 'limit') { const count = vi.mocked(invoke).mock.calls.length; await advance(2000); expect(invoke).toHaveBeenCalledTimes(count); }
    else expect(view.container.querySelector('.capture-scroll-border')).toHaveAttribute('data-interrupted', 'true');
    view.unmount();
  });
  it('waits for the last request before finishing and deduplicates pointer, local key and native key completion', async () => {
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    const onFinish = vi.fn().mockResolvedValue(true);
    const view = render(<ScrollCapture initial={initial} screen={screen} onFinish={onFinish} onCancel={vi.fn()} />);
    await flush(); fireEvent.click(overlay(view)); fireEvent.keyDown(window, { key: 'Enter' });
    act(() => { vi.mocked(listen).mock.calls[0]![1]({ event: 'popper:capture-scroll-finish', id: 1, payload: null }); });
    expect(onFinish).not.toHaveBeenCalled();
    await act(async () => { resolve(progress); }); expect(onFinish).toHaveBeenCalledTimes(1);
    await advance(2000); expect(invoke).toHaveBeenCalledTimes(1); view.unmount();
  });
  it.each(['cancel', 'unmount'] as const)('discards a queued completion and late results after %s', async (action) => {
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    const onFinish = vi.fn(), onCancel = vi.fn();
    const view = render(<ScrollCapture initial={initial} screen={screen} onFinish={onFinish} onCancel={onCancel} />);
    fireEvent.click(overlay(view));
    if (action === 'cancel') { fireEvent.keyDown(window, { key: 'Escape' }); fireEvent.contextMenu(overlay(view)); } else view.unmount();
    await act(async () => { resolve(progress); }); await advance(1000);
    expect(onFinish).not.toHaveBeenCalled(); expect(URL.createObjectURL).not.toHaveBeenCalled();
    if (action === 'cancel') { expect(onCancel).toHaveBeenCalledOnce(); view.unmount(); }
  });
  it('discards a binary preview arriving after cancellation', async () => {
    let resolve!: (value: ArrayBuffer) => void;
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_scroll_preview' ? new Promise((done) => { resolve = done; }) : progress);
    const view = render(<ScrollCapture initial={initial} screen={screen} onFinish={vi.fn()} onCancel={vi.fn()} />); await flush();
    fireEvent.keyDown(window, { key: 'Escape' }); view.unmount(); await act(async () => { resolve(packet()); });
    expect(URL.createObjectURL).not.toHaveBeenCalled();
  });
  it('retains captured content on source errors and allows retrying completion with Enter', async () => {
    const onFinish = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    const view = render(<ScrollCapture initial={initial} screen={screen} onFinish={onFinish} onCancel={vi.fn()} />); await flush();
    vi.mocked(invoke).mockRejectedValue(new Error('window covered')); await advance();
    expect(view.getByRole('status')).toHaveTextContent('window covered'); expect(view.getByRole('img')).toBeInTheDocument();
    await act(async () => { fireEvent.keyDown(window, { key: 'Enter' }); });
    expect(view.getByRole('status')).toHaveTextContent('重试完成');
    const count = vi.mocked(invoke).mock.calls.length; await advance(2000); expect(invoke).toHaveBeenCalledTimes(count);
    await act(async () => { fireEvent.keyDown(window, { key: 'Enter' }); }); expect(onFinish).toHaveBeenCalledTimes(2); view.unmount();
  });
  it('disposes a native completion listener that resolves after unmount', async () => {
    let resolve!: (dispose: () => void) => void;
    vi.mocked(listen).mockImplementation(() => new Promise((done) => { resolve = done; }));
    const view = render(<ScrollCapture initial={null} screen={screen} onFinish={vi.fn()} onCancel={vi.fn()} />);
    view.unmount(); const dispose = vi.fn(); await act(async () => { resolve(dispose); }); expect(dispose).toHaveBeenCalledOnce();
  });
});

describe('binary long-image preview', () => {
  it('decodes atomic metadata and keeps the PNG binary', () => {
    const decoded = decodeScrollPreview(packet()); expect(decoded.progress).toEqual(progress);
    expect(Array.from(new Uint8Array(decoded.png))).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
  });
  it('rejects truncated metadata and viewports outside the document', () => {
    expect(() => decodeScrollPreview(new ArrayBuffer(3))).toThrow('不完整');
    const bytes = packet(); new DataView(bytes).setUint32(0, 5000, true);
    expect(() => decodeScrollPreview(bytes)).toThrow('不完整');
    expect(() => decodeScrollPreview(packet({ ...progress, viewport: { ...progress.viewport, y: 1 } }))).toThrow('尺寸无效');
  });
});
