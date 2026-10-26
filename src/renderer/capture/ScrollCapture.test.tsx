import { act, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import type { ScrollCaptureProgress, ScrollCaptureSession } from '../../shared/capture';
import { ScrollCapture } from './ScrollCapture';
import { decodeScrollPreview } from './scrollPreview';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const screen = { x: -1920, y: -1080, width: 1920, height: 1080 };
const progress: ScrollCaptureProgress = { width: 800, height: 600, revision: 1, viewport: { x: 0, y: 0, width: 800, height: 600 }, status: 'unchanged' };
const initial: ScrollCaptureSession = { progress, layout: { selection: { x: 100, y: 100, width: 800, height: 600 }, panel: { x: 924, y: 100, width: 324, height: 765 }, toolbar: { x: 492, y: 712, width: 408, height: 72 } } };
function packet(state = progress, originY?: number): ArrayBuffer {
  const metadata = new TextEncoder().encode(JSON.stringify({ ...state, ...(originY === undefined ? {} : { originY }) }));
  const bytes = new Uint8Array(4 + metadata.length + 8);
  new DataView(bytes.buffer).setUint32(0, metadata.length, true);
  bytes.set(metadata, 4); bytes.set([137, 80, 78, 71, 13, 10, 26, 10], 4 + metadata.length);
  return bytes.buffer;
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(invoke).mockReset().mockImplementation(async (command) => command === 'capture_scroll_preview' ? packet() : progress);
  let urls = 0;
  vi.stubGlobal('URL', { createObjectURL: vi.fn().mockImplementation(() => `blob:preview-${++urls}`), revokeObjectURL: vi.fn() });
  Object.defineProperty(HTMLImageElement.prototype, 'decode', { configurable: true, value: vi.fn().mockResolvedValue(undefined) });
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); Reflect.deleteProperty(HTMLImageElement.prototype, 'decode'); });
const advance = async (ms = 180): Promise<void> => { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); };
const overlay = (view: ReturnType<typeof render>): HTMLElement => view.getByLabelText('滚动截图预览');
const flush = async (): Promise<void> => { await act(async () => {}); };

describe('manual scrolling overlay', () => {
  it('serializes preview requests after initialization', async () => {
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={vi.fn()} onCancel={vi.fn()} />);
    await advance(2000); expect(invoke).toHaveBeenCalledTimes(1);
    await act(async () => { resolve(progress); });
    expect(invoke).toHaveBeenCalledWith('capture_scroll_step'); expect(invoke).toHaveBeenCalledWith('capture_scroll_preview', expect.objectContaining({ maxWidth: expect.any(Number), maxHeight: expect.any(Number) }));
    expect(view.queryByText(/帧/)).toBeNull(); expect(view.getAllByRole('button')).toHaveLength(4); view.unmount();
  });
  it('shows the selection, bottom-right toolbar and viewport from one preview snapshot', async () => {
    const snapshot = { ...progress, height: 2400, revision: 4, viewport: { ...progress.viewport, y: 1200 } };
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_scroll_preview' ? packet(snapshot) : progress);
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={vi.fn()} onCancel={vi.fn()} />); await flush();
    expect(view.getByRole('img', { name: '已截取的长图' })).toHaveAttribute('src', 'blob:preview-1');
    const marker = view.getByLabelText('当前页面位置');
    const fit = Math.min((initial.layout.panel.width * innerWidth / screen.width - 26) / snapshot.width, (initial.layout.panel.height * innerHeight / screen.height - 134) / snapshot.height);
    expect(parseFloat(marker.style.top)).toBeCloseTo(1200 * fit); expect(parseFloat(marker.style.height)).toBeCloseTo(600 * fit);
    const border = view.container.querySelector<HTMLElement>('.capture-scroll-border')!;
    expect(parseFloat(border.style.left)).toBeCloseTo(100 / 1920 * 100); expect(parseFloat(border.style.top)).toBeCloseTo(100 / 1080 * 100);
    const toolbar = view.getByRole('toolbar', { name: '滚动截图操作' });
    expect(parseFloat(toolbar.style.top)).toBeCloseTo(712 / 1080 * 100);
    expect(parseFloat(toolbar.style.left) + parseFloat(toolbar.style.width)).toBeCloseTo(900 / 1920 * 100);
    view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:preview-1');
  });
  it('ignores outside clicks, preview clicks and Enter as completion triggers', async () => {
    const onAction = vi.fn();
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={onAction} onCancel={vi.fn()} />); await flush();
    fireEvent.click(overlay(view)); fireEvent.click(view.getByRole('img')); fireEvent.keyDown(window, { key: 'Enter' }); await advance();
    expect(onAction).not.toHaveBeenCalled(); expect(overlay(view)).toHaveAttribute('aria-busy', 'false'); view.unmount();
  });
  it.each([['保存 PNG', 'file'], ['创建贴图', 'pin'], ['问 AI', 'ask'], ['复制截图', 'clipboard']])('ends capture through the %s action', async (label, action) => {
    const onAction = vi.fn().mockResolvedValue(true);
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={onAction} onCancel={vi.fn()} />); await flush();
    fireEvent.click(view.getByLabelText(label!)); fireEvent.click(view.getByLabelText(label!)); await flush();
    expect(onAction).toHaveBeenCalledExactlyOnceWith(action);
    expect(view.getAllByRole('button').every(button => button.hasAttribute('disabled'))).toBe(true); view.unmount();
  });
  it('refreshes previews only when the document changes and releases replaced images', async () => {
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={vi.fn()} onCancel={vi.fn()} />); await advance(540);
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'capture_scroll_preview')).toHaveLength(1);
    const next = { ...progress, revision: 2, status: 'tracked' as const };
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_scroll_preview' ? packet(next) : next);
    await advance(); expect(URL.createObjectURL).toHaveBeenCalledTimes(2); expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
    view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
  });
  it('requests display-density previews and refreshes an unchanged document after resizing', async () => {
    vi.stubGlobal('devicePixelRatio', 2);
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={vi.fn()} onCancel={vi.fn()} />); await flush();
    const requests = (): unknown[][] => vi.mocked(invoke).mock.calls.filter(([command]) => command === 'capture_scroll_preview');
    expect(requests()[0]?.[1]).toEqual({ maxWidth: Math.ceil((324 * innerWidth / 1920 - 26) * 4), maxHeight: Math.ceil((765 * innerHeight / 1080 - 134) * 4) });
    vi.stubGlobal('innerWidth', 3840); vi.stubGlobal('innerHeight', 2160);
    fireEvent(window, new Event('resize')); await advance(50);
    expect(requests()).toHaveLength(2); expect(requests()[1]?.[1]).toEqual({ maxWidth: 1024, maxHeight: 2048 });
    view.unmount();
  });
  it('publishes a new image and its geometry only after decoding, keeping the previous image alive', async () => {
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={vi.fn()} onCancel={vi.fn()} />); await flush();
    let resolve!: () => void;
    vi.mocked(HTMLImageElement.prototype.decode).mockImplementationOnce(() => new Promise<void>((done) => { resolve = done; }));
    const next = { ...progress, height: 2400, revision: 2, viewport: { ...progress.viewport, y: 1200 } };
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_scroll_preview' ? packet(next) : next);
    const old = view.getByRole('img').getAttribute('src'); const top = view.getByLabelText('当前页面位置').style.top;
    await advance(50); expect(resolve).toBeTypeOf('function');
    expect(view.getByRole('img')).toHaveAttribute('src', old); expect(view.getByLabelText('当前页面位置').style.top).toBe(top);
    expect(URL.revokeObjectURL).not.toHaveBeenCalled();
    await act(async () => { resolve(); });
    expect(view.getByRole('img')).toHaveAttribute('src', 'blob:preview-2'); expect(URL.revokeObjectURL).toHaveBeenCalledExactlyOnceWith(old);
    view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
  });
  it('keeps the last image on decode failure and releases the failed replacement', async () => {
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={vi.fn()} onCancel={vi.fn()} />); await flush();
    vi.mocked(HTMLImageElement.prototype.decode).mockRejectedValueOnce(new Error('invalid preview'));
    const next = { ...progress, revision: 2 };
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_scroll_preview' ? packet(next) : next);
    await advance(50);
    expect(view.getByRole('img')).toHaveAttribute('src', 'blob:preview-1'); expect(view.getByRole('status')).toHaveTextContent('invalid preview');
    expect(URL.revokeObjectURL).toHaveBeenCalledExactlyOnceWith('blob:preview-2'); view.unmount();
  });
  it.each(['output', 'cancel', 'unmount'] as const)('aborts pending decoding promptly on %s and discards its late completion', async (action) => {
    let resolve!: () => void;
    vi.mocked(HTMLImageElement.prototype.decode).mockImplementationOnce(() => new Promise<void>((done) => { resolve = done; }));
    const onAction = vi.fn().mockResolvedValue(true), onCancel = vi.fn();
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={onAction} onCancel={onCancel} />); await flush();
    expect(resolve).toBeTypeOf('function');
    if (action === 'output') fireEvent.click(view.getByLabelText('复制截图'));
    else if (action === 'cancel') fireEvent.keyDown(window, { key: 'Escape' });
    else view.unmount();
    await flush(); expect(URL.revokeObjectURL).toHaveBeenCalledExactlyOnceWith('blob:preview-1');
    if (action === 'output') expect(onAction).toHaveBeenCalledExactlyOnceWith('clipboard');
    if (action === 'cancel') expect(onCancel).toHaveBeenCalledOnce();
    await act(async () => { resolve(); }); expect(view.queryByRole('img')).toBeNull();
    if (action !== 'unmount') view.unmount();
  });
  it.each(['unmatched', 'limit'] as const)('preserves the preview and output actions on %s', async (status) => {
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={vi.fn()} onCancel={vi.fn()} />); await flush();
    vi.mocked(invoke).mockResolvedValue({ ...progress, status }); await advance();
    expect(view.getByRole('status')).toHaveTextContent(status === 'limit' ? '容量上限' : '滚回已截取的位置');
    expect(view.getByRole('img')).toBeInTheDocument(); expect(view.getByLabelText('复制截图')).toBeEnabled();
    if (status === 'limit') { const count = vi.mocked(invoke).mock.calls.length; await advance(2000); expect(invoke).toHaveBeenCalledTimes(count); }
    else expect(view.container.querySelector('.capture-scroll-border')).toHaveAttribute('data-interrupted', 'true'); view.unmount();
  });
  it('waits for an in-flight preview request before executing the chosen action', async () => {
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    const onAction = vi.fn().mockResolvedValue(true);
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={onAction} onCancel={vi.fn()} />);
    fireEvent.click(view.getByLabelText('复制截图')); expect(onAction).not.toHaveBeenCalled();
    await act(async () => { resolve(progress); }); expect(onAction).toHaveBeenCalledExactlyOnceWith('clipboard');
    await advance(2000); expect(invoke).toHaveBeenCalledTimes(1); view.unmount();
  });
  it.each(['cancel', 'unmount'] as const)('discards queued output and late results after %s', async (action) => {
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    const onAction = vi.fn(), onCancel = vi.fn();
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={onAction} onCancel={onCancel} />);
    fireEvent.click(view.getByLabelText('保存 PNG'));
    if (action === 'cancel') { fireEvent.keyDown(window, { key: 'Escape' }); fireEvent.contextMenu(overlay(view)); } else view.unmount();
    await act(async () => { resolve(progress); }); await advance(1000);
    expect(onAction).not.toHaveBeenCalled(); expect(URL.createObjectURL).not.toHaveBeenCalled();
    if (action === 'cancel') { expect(onCancel).toHaveBeenCalledOnce(); view.unmount(); }
  });
  it('discards a binary preview arriving after cancellation', async () => {
    let resolve!: (value: ArrayBuffer) => void;
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_scroll_preview' ? new Promise((done) => { resolve = done; }) : progress);
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={vi.fn()} onCancel={vi.fn()} />); await flush();
    fireEvent.keyDown(window, { key: 'Escape' }); view.unmount(); await act(async () => { resolve(packet()); }); expect(URL.createObjectURL).not.toHaveBeenCalled();
  });
  it('retains content on output failure or save cancellation and permits another action without resuming polling', async () => {
    const onAction = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    const view = render(<ScrollCapture initial={initial} screen={screen} onAction={onAction} onCancel={vi.fn()} />); await flush();
    vi.mocked(invoke).mockRejectedValue(new Error('window covered')); await advance();
    expect(view.getByRole('status')).toHaveTextContent('window covered'); expect(view.getByRole('img')).toBeInTheDocument();
    await act(async () => { fireEvent.click(view.getByLabelText('保存 PNG')); }); expect(view.getByRole('status')).toHaveTextContent('内容已保留');
    const count = vi.mocked(invoke).mock.calls.length; await advance(2000); expect(invoke).toHaveBeenCalledTimes(count);
    await act(async () => { fireEvent.click(view.getByLabelText('复制截图')); });
    expect(onAction.mock.calls).toEqual([['file'], ['clipboard']]); view.unmount();
  });
});

describe('binary long-image preview', () => {
  it('decodes atomic metadata and keeps the PNG binary', () => {
    const decoded = decodeScrollPreview(packet()); expect(decoded.progress).toEqual(progress);
    expect(decoded.originY).toBe(0);
    expect(Array.from(new Uint8Array(decoded.png))).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
  });
  it('keeps the document origin for upward growth and rejects invalid offsets', () => {
    expect(decodeScrollPreview(packet(progress, -600)).originY).toBe(-600);
    expect(() => decodeScrollPreview(packet(progress, 0.5))).toThrow('尺寸无效');
    expect(() => decodeScrollPreview(packet(progress, Number.MAX_SAFE_INTEGER + 1))).toThrow('尺寸无效');
  });
  it('rejects truncated metadata and viewports outside the document', () => {
    expect(() => decodeScrollPreview(new ArrayBuffer(3))).toThrow('不完整');
    const bytes = packet(); new DataView(bytes).setUint32(0, 5000, true); expect(() => decodeScrollPreview(bytes)).toThrow('不完整');
    expect(() => decodeScrollPreview(packet({ ...progress, viewport: { ...progress.viewport, y: 1 } }))).toThrow('尺寸无效');
  });
});
