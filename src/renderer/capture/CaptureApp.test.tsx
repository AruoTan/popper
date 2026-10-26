import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { decodePng, pngBytes } from './drawing';
import { CaptureApp } from './CaptureApp';
import type { CaptureHitTest, ScrollCaptureProgress, ScrollCaptureSession } from '../../shared/capture';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('./drawing', async (importOriginal) => ({ ...await importOriginal<typeof import('./drawing')>(), decodePng: vi.fn(), renderScene: vi.fn(), pngBytes: vi.fn() }));
const meta = { id: 'session', screen: { x: -1920, y: 0, width: 1920, height: 1080 }, scale: 1.5, color: '#ff4545', lineWidth: 3 };
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_session' ? meta : command === 'capture_image' ? new ArrayBuffer(0) : undefined);
});
afterEach(() => { vi.unstubAllGlobals(); Reflect.deleteProperty(HTMLImageElement.prototype, 'decode'); });
describe('scroll capture integration', () => {
  const rect = { x: 20, y: 20, width: 800, height: 600 };
  const progress: ScrollCaptureProgress = { width: 800, height: 600, revision: 1, viewport: { x: 0, y: 0, width: 800, height: 600 }, status: 'unchanged' };
  const scrolling: ScrollCaptureSession = { progress, layout: { selection: rect, panel: { x: 900, y: 20, width: 324, height: 765 }, toolbar: { x: 412, y: 632, width: 408, height: 72 } } };
  const calls = (command: string): number => vi.mocked(invoke).mock.calls.filter(([name]) => name === command).length;
  beforeEach(() => {
    vi.stubGlobal('URL', { createObjectURL: vi.fn().mockReturnValue('blob:preview'), revokeObjectURL: vi.fn() });
    Object.defineProperty(HTMLImageElement.prototype, 'decode', { configurable: true, value: vi.fn().mockResolvedValue(undefined) });
    vi.stubGlobal('PointerEvent', MouseEvent);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.mocked(decodePng).mockResolvedValue(new Image());
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return { ...meta, pointer: { x: 100, y: 100 }, targets: [{ rect, label: 'page', controls: [] }] };
      if (command === 'capture_image' || command === 'capture_crop') return new ArrayBuffer(0);
      if (command === 'capture_scroll_begin') return scrolling;
      if (command === 'capture_scroll_step') return progress;
      if (command === 'capture_scroll_preview') {
        const metadata = new TextEncoder().encode(JSON.stringify(progress));
        const bytes = new Uint8Array(4 + metadata.length + 8);
        new DataView(bytes.buffer).setUint32(0, metadata.length, true); bytes.set(metadata, 4); return bytes.buffer;
      }
      if (command === 'capture_scroll_finish') return { width: 800, height: 2400, version: 7 };
      if (command === 'capture_export') return false;
    });
  });
  async function selected(): Promise<ReturnType<typeof render>> {
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    const main = view.container.querySelector('main')!;
    Object.assign(main, { setPointerCapture: vi.fn(), hasPointerCapture: () => false });
    const at = { clientX: 100 * window.innerWidth / meta.screen.width, clientY: 100 * window.innerHeight / meta.screen.height, button: 0 };
    fireEvent.pointerDown(main, at); fireEvent.pointerUp(main, at);
    await waitFor(() => expect(view.getByLabelText('滚动截图')).toBeEnabled()); return view;
  }
  async function started(): Promise<ReturnType<typeof render>> {
    const view = await selected(); fireEvent.click(view.getByLabelText('滚动截图'));
    await waitFor(() => expect(view.getByRole('toolbar', { name: '滚动截图操作' })).toBeInTheDocument()); return view;
  }
  it('hands a normal screenshot directly to Quick Ask and ignores focus loss during delivery', async () => {
    let complete!: (value: boolean) => void;
    const bytes = new Uint8Array([7, 8, 9]);
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((command, ...args) => {
      if (command === 'capture_crop') return Promise.resolve(bytes.buffer);
      if (command === 'capture_export') return new Promise((done) => { complete = done; });
      return implementation(command, ...args);
    });
    const view = await selected();
    fireEvent.click(view.getByLabelText('问 AI'));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_export', bytes, { headers: { 'x-capture-version': '1', 'x-capture-output': 'ask' } }));
    fireEvent.blur(window);
    await act(async () => { await new Promise((done) => setTimeout(done, 150)); });
    expect(calls('capture_cancel')).toBe(0);
    expect(view.getByLabelText('问 AI')).toBeDisabled();
    await act(async () => complete(true));
    expect(calls('capture_scroll_begin')).toBe(0);
    expect(calls('capture_export')).toBe(1);
    view.unmount();
  });
  it('hands off a cropped base image and independently translated annotations when creating a pin', async () => {
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((command, ...args) => command === 'capture_crop'
      ? Promise.resolve(new Uint8Array([7, 8, 9]).buffer) : implementation(command, ...args));
    const view = await selected(), main = view.container.querySelector('main')!;
    fireEvent.click(view.getByLabelText('矩形'));
    const point = (x: number, y: number) => ({ clientX: x * window.innerWidth / meta.screen.width, clientY: y * window.innerHeight / meta.screen.height, button: 0 });
    fireEvent.pointerDown(main, point(60, 70));
    fireEvent.pointerMove(main, point(160, 170));
    fireEvent.pointerUp(main, point(160, 170));
    fireEvent.click(view.getByLabelText('创建贴图'));
    await waitFor(() => expect(calls('capture_export')).toBe(1));
    const args = vi.mocked(invoke).mock.calls.find(([command]) => command === 'capture_crop')![1] as { rect: object; annotations: { points: { x: number; y: number }[] }[] };
    expect(args.rect).toEqual(rect);
    expect(args.annotations).toHaveLength(1);
    expect(args.annotations[0]!.points[0]!.x).toBeCloseTo(40);
    expect(args.annotations[0]!.points[0]!.y).toBeCloseTo(50);
    expect(args.annotations[0]!.points[1]!.x).toBeCloseTo(140);
    expect(args.annotations[0]!.points[1]!.y).toBeCloseTo(150);
    expect(invoke).toHaveBeenCalledWith('capture_export', new Uint8Array([7, 8, 9]), expect.objectContaining({ headers: expect.objectContaining({ 'x-capture-output': 'pin' }) }));
    expect(pngBytes).not.toHaveBeenCalled();
    view.unmount();
  });
  it('keeps the frozen canvas and selection until the final scrolling layout is ready', async () => {
    let resolve!: (value: ScrollCaptureSession) => void;
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, ...args) => command === 'capture_scroll_begin' ? new Promise((done) => { resolve = done; }) : implementation(command, ...args));
    const view = await selected();
    const scene = view.container.querySelector('.capture-scene');
    const selection = view.getByLabelText('截图选区');
    const style = selection.getAttribute('style');
    fireEvent.click(view.getByLabelText('滚动截图'));
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    expect(view.container.querySelector('.capture-scene')).toBe(scene);
    expect(view.getByLabelText('截图选区')).toBe(selection);
    expect(selection).toHaveAttribute('style', style);
    expect(view.queryByLabelText('滚动截图预览')).toBeNull();
    expect(view.getByLabelText('滚动截图')).toBeDisabled();
    fireEvent.click(view.getByLabelText('滚动截图')); expect(calls('capture_scroll_begin')).toBe(1);
    fireEvent.blur(window); await act(async () => { await new Promise((done) => setTimeout(done, 150)); });
    expect(calls('capture_cancel')).toBe(0);
    await act(async () => { resolve(scrolling); });
    const border = view.container.querySelector<HTMLElement>('.capture-scroll-border')!;
    expect(parseFloat(border.style.left)).toBeCloseTo(rect.x / meta.screen.width * 100);
    expect(parseFloat(border.style.top)).toBeCloseTo(rect.y / meta.screen.height * 100);
    expect(view.getByRole('toolbar', { name: '滚动截图操作' }).querySelectorAll('button')).toHaveLength(4);
    expect(view.queryByLabelText('滚动截图')).toBeNull(); view.unmount();
  });
  it('allows source-window focus and ignores outside clicks and Enter while capture continues', async () => {
    const view = await started(); expect(invoke).toHaveBeenCalledWith('capture_scroll_begin', { rect, version: 1 });
    expect(Array.from(view.getByRole('toolbar').querySelectorAll('button'), (button) => button.textContent)).toEqual(['保存', '贴图', '问 AI', '复制']);
    expect(view.queryByLabelText('截图选区')).toBeNull();
    fireEvent.blur(window); await act(async () => { await new Promise((done) => setTimeout(done, 150)); });
    fireEvent.click(view.getByLabelText('滚动截图预览')); fireEvent.keyDown(window, { key: 'Enter' });
    expect(invoke).not.toHaveBeenCalledWith('capture_cancel'); expect(invoke).not.toHaveBeenCalledWith('capture_scroll_finish'); view.unmount();
  });
  it.each([['保存 PNG', 'file'], ['创建贴图', 'pin'], ['问 AI', 'ask'], ['复制截图', 'clipboard']])('finishes and executes %s without entering another stage or decoding the long image', async (label, output) => {
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, ...args) => command === 'capture_export' ? true : implementation(command, ...args));
    const view = await started(); fireEvent.click(view.getByLabelText(label!));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_export', expect.any(Uint8Array), expect.objectContaining({ headers: { 'x-capture-version': '7', 'x-capture-output': output } })));
    expect(calls('capture_scroll_finish')).toBe(1); expect(calls('capture_show')).toBe(1); expect(calls('capture_crop')).toBe(0);
    expect(decodePng).toHaveBeenCalledTimes(1); expect(view.container.querySelector('.capture-scene')).toBeNull();
    expect(view.getByLabelText('复制截图')).toBeDisabled(); view.unmount();
  });
  it('retains the stopped toolbar after save cancellation and changes output without recomposing', async () => {
    const view = await started(); fireEvent.click(view.getByLabelText('保存 PNG'));
    await waitFor(() => expect(view.getByRole('status')).toHaveTextContent('内容已保留'));
    fireEvent.click(view.getByLabelText('复制截图'));
    await waitFor(() => expect(calls('capture_export')).toBe(2)); expect(calls('capture_scroll_finish')).toBe(1); expect(calls('capture_show')).toBe(1);
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'capture_export').map(([, , options]) => options)).toEqual([
      { headers: { 'x-capture-version': '7', 'x-capture-output': 'file' } }, { headers: { 'x-capture-version': '7', 'x-capture-output': 'clipboard' } },
    ]); view.unmount();
  });
  it('retries a failed direct output with the same committed long image', async () => {
    let exports = 0;
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, ...args) => {
      if (command === 'capture_export') { if (++exports === 1) throw new Error('clipboard busy'); return true; }
      return implementation(command, ...args);
    });
    const view = await started(); fireEvent.click(view.getByLabelText('复制截图'));
    await waitFor(() => expect(view.getByRole('alert')).toHaveTextContent('clipboard busy'));
    await waitFor(() => expect(view.getByLabelText('复制截图')).toBeEnabled()); fireEvent.click(view.getByLabelText('复制截图'));
    await waitFor(() => expect(calls('capture_export')).toBe(2));
    expect(calls('capture_scroll_finish')).toBe(1); expect(calls('capture_show')).toBe(1);
    expect(calls('capture_crop')).toBe(0); expect(decodePng).toHaveBeenCalledTimes(1); view.unmount();
  });
  it('returns to ordinary screenshot editing when scrolling setup fails', async () => {
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, ...args) => { if (command === 'capture_scroll_begin') throw new Error('capture exclusion unavailable'); return implementation(command, ...args); });
    const view = await selected(); const scene = view.container.querySelector('.capture-scene'); fireEvent.click(view.getByLabelText('滚动截图'));
    await waitFor(() => expect(view.getByRole('alert')).toHaveTextContent('capture exclusion unavailable'));
    expect(view.container.querySelector('.capture-scene')).toBe(scene);
    expect(view.getByLabelText('复制截图')).toBeEnabled(); expect(view.queryByLabelText('滚动截图预览')).toBeNull(); view.unmount();
  });
  it('discards a late scrolling setup result after Escape', async () => {
    let resolve!: (value: ScrollCaptureSession) => void;
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, ...args) => command === 'capture_scroll_begin' ? new Promise((done) => { resolve = done; }) : implementation(command,...args));
    const view = await selected(); fireEvent.click(view.getByLabelText('滚动截图'));
    await waitFor(() => expect(resolve).toBeTypeOf('function')); fireEvent.keyDown(window, { key: 'Escape' }); await act(async () => { resolve(scrolling); });
    expect(invoke).toHaveBeenCalledWith('capture_cancel'); expect(calls('capture_scroll_step')).toBe(0); expect(view.queryByLabelText('滚动截图预览')).toBeNull(); view.unmount();
  });
  it('discards a completed document after Escape instead of executing the pending output', async () => {
    let resolve!: (value: { width: number; height: number; version: number }) => void;
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, ...args) => command === 'capture_scroll_finish' ? new Promise((done) => { resolve = done; }) : implementation(command, ...args));
    const view = await started(); fireEvent.click(view.getByLabelText('复制截图')); await waitFor(() => expect(resolve).toBeTypeOf('function'));
    fireEvent.keyDown(window, { key: 'Escape' }); await act(async () => { resolve({ width: 800, height: 2400, version: 7 }); });
    expect(calls('capture_cancel')).toBe(1); expect(calls('capture_export')).toBe(0); expect(calls('capture_image')).toBe(1); view.unmount();
  });
  it('retries image delivery without composing or committing the long document twice', async () => {
    let imageRequests = 0;
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, ...args) => { if (command === 'capture_image' && ++imageRequests === 2) throw new Error('transient image transport'); return implementation(command, ...args); });
    const view = await started(); fireEvent.click(view.getByLabelText('复制截图'));
    await waitFor(() => expect(view.getByRole('alert')).toHaveTextContent('transient image transport'));
    await waitFor(() => expect(view.getByLabelText('复制截图')).toBeEnabled()); fireEvent.click(view.getByLabelText('复制截图'));
    await waitFor(() => expect(calls('capture_export')).toBe(1)); expect(calls('capture_scroll_finish')).toBe(1); expect(calls('capture_show')).toBe(1); view.unmount();
  });
});
describe('capture initialization cleanup', () => {
  it('restores the desktop through the backend when image initialization fails', async () => {
    vi.mocked(decodePng).mockRejectedValueOnce(new Error('无法解码屏幕图像'));
    render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_fail', { message: '无法解码屏幕图像' }));
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'capture_show')).toBe(false);
  });
  it('discards a failed decode from an unmounted session', async () => {
    let reject!: (error: Error) => void;
    vi.mocked(decodePng).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
    const view = render(<CaptureApp />);
    await waitFor(() => expect(decodePng).toHaveBeenCalled());
    view.unmount(); reject(new Error('late decode'));
    await Promise.resolve(); await Promise.resolve();
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'capture_fail' || command === 'capture_show')).toBe(false);
  });
});

describe('capture hover integration', () => {
  it.each([[0.25, 0.25], [0.25, 0.75], [0.75, 0.25], [0.75, 0.75]])('positions the magnifier by quadrant from startup at (%s, %s) through movement and resizing', async (initialX, initialY) => {
    vi.stubGlobal('PointerEvent', MouseEvent);
    vi.stubGlobal('innerWidth', 1000); vi.stubGlobal('innerHeight', 700);
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ width: 159, height: 197 } as DOMRect);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.mocked(decodePng).mockResolvedValueOnce(new Image());
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_session' ? { ...meta, pointer: { x: initialX * meta.screen.width, y: initialY * meta.screen.height } } : command === 'capture_image' ? new ArrayBuffer(0) : undefined);
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    const main = view.container.querySelector('main')!;
    const panel = view.container.querySelector<HTMLElement>('.capture-magnifier')!;
    const checkPosition = (x: number, y: number): void => {
      const left = Number.parseFloat(panel.style.left), top = Number.parseFloat(panel.style.top);
      expect(left).toBeGreaterThanOrEqual(8);
      expect(top).toBeGreaterThanOrEqual(8);
      expect(left + 159).toBeLessThanOrEqual(window.innerWidth - 8);
      expect(top + 197).toBeLessThanOrEqual(window.innerHeight - 8);
      if (x >= window.innerWidth / 2) expect(left + 159).toBeLessThan(x);
      else expect(left).toBeGreaterThan(x);
      if (y >= window.innerHeight / 2) expect(top + 197).toBeLessThan(y);
      else expect(top).toBeGreaterThan(y);
    };
    checkPosition(initialX * window.innerWidth, initialY * window.innerHeight);
    for (const [x, y] of [[10, 10], [990, 10], [10, 690], [990, 690], [490, 340], [490, 360], [510, 340], [510, 360], [500, 350]] as const) {
      fireEvent.pointerMove(main, { clientX: x, clientY: y });
      checkPosition(x, y);
    }
    vi.stubGlobal('innerWidth', 800); vi.stubGlobal('innerHeight', 600);
    fireEvent(window, new Event('resize'));
    fireEvent.pointerMove(main, { clientX: 790, clientY: 590 });
    checkPosition(790, 590);
    view.unmount();
  });
  it('keeps a Tab-selected parent when late point recognition adds smaller controls', async () => {
    vi.stubGlobal('PointerEvent', MouseEvent);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.mocked(decodePng).mockResolvedValueOnce(new Image());
    const control = { rect: { x: 10, y: 10, width: 200, height: 150 }, label: 'control' };
    const root = { rect: { x: 0, y: 0, width: 1920, height: 1080 }, label: 'window', controls: [control] };
    let resolve!: (response: CaptureHitTest) => void;
    const pending = new Promise<CaptureHitTest>((done) => { resolve = done; });
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_session' ? { ...meta, targets: [root] } : command === 'capture_image' ? new ArrayBuffer(0) : command === 'capture_candidates' ? pending : undefined);
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    fireEvent.pointerMove(view.container.querySelector('main')!, { clientX: 20, clientY: 20 });
    await waitFor(() => expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'capture_candidates')).toBe(true));
    fireEvent.keyDown(window, { key: 'Tab' });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('1920 × 1080');
    await act(async () => { resolve({ candidates: [{ rect: { x: 15, y: 15, width: 40, height: 40 }, label: 'text' }, control, root], pending: false }); });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('1920 × 1080');
    fireEvent.keyDown(window, { key: 'Tab', shiftKey: true });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('200 × 150');
    fireEvent.pointerMove(view.container.querySelector('main')!, { clientX: 21, clientY: 22 });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('200 × 150');
    view.unmount();
  });
  it('shows the captured cursor target without requiring an initial mouse movement', async () => {
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.mocked(decodePng).mockResolvedValueOnce(new Image());
    const control = { rect: { x: 10, y: 10, width: 200, height: 150 }, label: 'native control' };
    const targets = [{ rect: { x: 0, y: 0, width: 1920, height: 1080 }, label: 'window', controls: [control] }];
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_session' ? { ...meta, targets, pointer: { x: 20, y: 20 } } : command === 'capture_image' ? new ArrayBuffer(0) : command === 'capture_candidates' ? new Promise(() => {}) : undefined);
    const view = render(<CaptureApp />);
    await waitFor(() => expect(view.container.querySelector('.capture-size')).toHaveTextContent('200 × 150'));
    view.unmount();
  });
  it('highlights a native control before the first provider request completes', async () => {
    vi.stubGlobal('PointerEvent', MouseEvent);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.mocked(decodePng).mockResolvedValueOnce(new Image());
    const native = { rect: { x: 10, y: 10, width: 200, height: 150 }, label: 'native control' };
    const targets = [{ rect: { x: 0, y: 0, width: 1920, height: 1080 }, label: 'window', controls: [native] }];
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_session' ? { ...meta, targets } : command === 'capture_image' ? new ArrayBuffer(0) : command === 'capture_candidates' ? new Promise(() => {}) : undefined);
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    fireEvent.pointerMove(view.container.querySelector('main')!, { clientX: 20, clientY: 20 });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('200 × 150');
    view.unmount();
  });
  it('applies refined control bounds even when the pointer moved during the request', async () => {
    vi.stubGlobal('PointerEvent', MouseEvent);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.mocked(decodePng).mockResolvedValueOnce(new Image());
    let resolve!: (response: CaptureHitTest) => void;
    const pending = new Promise<CaptureHitTest>((done) => { resolve = done; });
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_session' ? { ...meta, targets: [] } : command === 'capture_image' ? new ArrayBuffer(0) : command === 'capture_candidates' ? pending : undefined);
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    const main = view.container.querySelector('main')!;
    fireEvent.pointerMove(main, { clientX: 20, clientY: 20 });
    await waitFor(() => expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'capture_candidates')).toBe(true));
    fireEvent.pointerMove(main, { clientX: 21, clientY: 22 });
    resolve({ candidates: [{ rect: { x: 10, y: 10, width: 120, height: 90 }, label: 'text' }], pending: false });
    await waitFor(() => expect(view.container.querySelector('.capture-size')).toHaveTextContent('120 × 90'));
    view.unmount();
  });
});

describe('capture keyboard pointer movement', () => {
  beforeEach(() => {
    vi.stubGlobal('PointerEvent', MouseEvent);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.mocked(decodePng).mockResolvedValueOnce(new Image());
  });

  it('moves one physical pixel in every direction and synchronizes magnifier sampling and control feedback', async () => {
    let point = { x: 100, y: 100 };
    const context = {
      fillRect: vi.fn(), drawImage: vi.fn(), beginPath: vi.fn(), moveTo: vi.fn(), lineTo: vi.fn(), stroke: vi.fn(), strokeRect: vi.fn(),
      getImageData: vi.fn(() => ({ data: new Uint8ClampedArray([point.x, point.y, 0, 255]) })),
    };
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(context as unknown as CanvasRenderingContext2D);
    const controls = [100, 101].map((x) => ({ rect: { x, y: 100, width: 1, height: 1 }, label: `pixel ${x}` }));
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'capture_session') return { ...meta, pointer: point, targets: [{ rect: { x: 0, y: 0, width: 1920, height: 1080 }, label: 'window', controls }] };
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_move_pointer') {
        const { dx, dy } = args as { dx: number; dy: number };
        point = { x: point.x + dx, y: point.y + dy }; return point;
      }
      if (command === 'capture_candidates') return { candidates: [], pending: false };
    });
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    const panel = view.container.querySelector<HTMLElement>('.capture-magnifier')!;
    const initialLeft = Number.parseFloat(panel.style.left);
    for (const [key, x, y] of [['ArrowRight', 101, 100], ['ArrowDown', 101, 101], ['ArrowLeft', 100, 101], ['ArrowUp', 100, 100]] as const) {
      fireEvent.keyDown(window, { key });
      await waitFor(() => {
        expect(panel).toHaveTextContent(`${meta.screen.x + x}, ${y}`);
        expect(panel).toHaveTextContent(`#${x.toString(16)}${y.toString(16)}00`.toUpperCase());
        expect(context.drawImage.mock.lastCall?.slice(1, 3)).toEqual([x - 5, y - 5]);
        expect(Number.parseFloat(panel.style.left)).toBeCloseTo(initialLeft + (x - 100) * window.innerWidth / meta.screen.width);
        if (y === 100) expect(view.container.querySelector('.capture-size')).toHaveTextContent(`pixel ${x}`);
      });
    }
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'capture_move_pointer').map(([, args]) => args)).toEqual([
      { dx: 1, dy: 0 }, { dx: 0, dy: 1 }, { dx: -1, dy: 0 }, { dx: 0, dy: -1 },
    ]);
    view.unmount();
  });

  it('serializes repeated keys so every press moves the native pointer once', async () => {
    let resolve!: (point: { x: number; y: number }) => void;
    let calls = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return { ...meta, pointer: { x: 100, y: 100 } };
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_move_pointer') return ++calls === 1 ? new Promise((done) => { resolve = done; }) : { x: 102, y: 100 };
    });
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    fireEvent.keyDown(window, { key: 'ArrowRight' }); fireEvent.keyDown(window, { key: 'ArrowRight', repeat: true });
    await waitFor(() => expect(calls).toBe(1));
    await act(async () => { resolve({ x: 101, y: 100 }); });
    await waitFor(() => expect(view.container.querySelector('.capture-magnifier')).toHaveTextContent('-1818, 100'));
    expect(calls).toBe(2);
    view.unmount();
  });

  it('retains selection resizing shortcuts and leaves text input arrow keys alone', async () => {
    const rect = { x: 20, y: 20, width: 200, height: 100 };
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return { ...meta, pointer: { x: 50, y: 50 }, targets: [{ rect, label: 'window', controls: [] }] };
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_move_pointer') return { x: 51, y: 50 };
    });
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    const main = view.container.querySelector('main')!;
    Object.assign(main, { setPointerCapture: vi.fn(), hasPointerCapture: () => false });
    const at = { clientX: 50 * window.innerWidth / meta.screen.width, clientY: 50 * window.innerHeight / meta.screen.height };
    fireEvent.pointerDown(main, { ...at, button: 0 }); fireEvent.pointerUp(main, { ...at, button: 0 });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('200 × 100');
    fireEvent.keyDown(window, { key: 'ArrowRight' });
    await waitFor(() => expect(view.container.querySelector('.capture-magnifier')).toHaveTextContent('-1869, 50'));
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('200 × 100');
    fireEvent.keyDown(window, { key: 'ArrowRight', ctrlKey: true });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('201 × 100');
    fireEvent.keyDown(window, { key: 'ArrowDown', shiftKey: true });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('201 × 99');
    fireEvent.keyDown(view.getByLabelText('线宽'), { key: 'ArrowRight' });
    await act(async () => {});
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'capture_move_pointer')).toHaveLength(1);
    view.unmount();
  });

  it('ignores late movement responses and queued keys after cancellation', async () => {
    let resolve!: (point: { x: number; y: number }) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return { ...meta, pointer: { x: 100, y: 100 } };
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_move_pointer') return new Promise((done) => { resolve = done; });
    });
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    fireEvent.keyDown(window, { key: 'ArrowRight' }); fireEvent.keyDown(window, { key: 'ArrowDown' });
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    fireEvent.keyDown(window, { key: 'Escape' });
    await act(async () => { resolve({ x: 101, y: 100 }); });
    expect(invoke).toHaveBeenCalledWith('capture_cancel');
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'capture_move_pointer')).toHaveLength(1);
    expect(view.container.querySelector('.capture-magnifier')).toBeNull();
    view.unmount();
  });
});

describe('capture focus lifecycle', () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.mocked(decodePng).mockResolvedValue(new Image());
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return meta;
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_is_foreground') return true;
    });
  });
  afterEach(() => vi.useRealTimers());
  async function shown(): Promise<ReturnType<typeof render>> {
    const view = render(<CaptureApp />);
    await waitFor(() => expect(document.activeElement).toBe(view.container.querySelector('main')));
    vi.useFakeTimers();
    return view;
  }

  it('ignores startup blur while the frozen image is still decoding', async () => {
    let resolve!: (image: HTMLImageElement) => void;
    vi.mocked(decodePng).mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    const view = render(<CaptureApp />);
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    fireEvent.blur(window);
    expect(invoke).not.toHaveBeenCalledWith('capture_cancel');
    await act(async () => { resolve(new Image()); });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    view.unmount();
  });

  it('survives blur emitted while the native frame and WebView acquire focus', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return meta;
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_show') { fireEvent.focus(window); fireEvent.blur(window); }
    });
    const view = await shown();
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(invoke).not.toHaveBeenCalledWith('capture_cancel');
    expect(invoke).not.toHaveBeenCalledWith('capture_fail', expect.anything());
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(invoke).toHaveBeenCalledWith('capture_cancel');
    view.unmount();
  });

  it('keeps editing when DOM blur leaves the native capture window foreground', async () => {
    const view = await shown();
    fireEvent.blur(window);
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(invoke).toHaveBeenCalledWith('capture_is_foreground');
    expect(invoke).not.toHaveBeenCalledWith('capture_cancel');
    view.unmount();
  });

  it('cancels once after confirming focus moved to another application', async () => {
    const view = await shown();
    vi.mocked(invoke).mockResolvedValue(false);
    fireEvent.blur(window); fireEvent.blur(window);
    expect(invoke).not.toHaveBeenCalledWith('capture_cancel');
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'capture_cancel')).toHaveLength(1);
    view.unmount();
  });

  it('discards a foreground check after focus returns', async () => {
    const view = await shown();
    let resolve!: (foreground: boolean) => void;
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_is_foreground' ? new Promise((done) => { resolve = done; }) : undefined);
    fireEvent.blur(window);
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(resolve).toBeTypeOf('function');
    fireEvent.focus(window);
    await act(async () => { resolve(false); });
    expect(invoke).not.toHaveBeenCalledWith('capture_cancel');
    view.unmount();
  });

  it('keeps the session when foreground verification fails', async () => {
    const view = await shown();
    vi.mocked(invoke).mockRejectedValue(new Error('window unavailable'));
    fireEvent.blur(window);
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(invoke).not.toHaveBeenCalledWith('capture_cancel');
    view.unmount();
  });

  it('discards an old foreground check even after a cancelled save returns to editing', async () => {
    vi.stubGlobal('PointerEvent', MouseEvent);
    const view = await shown();
    let resolve!: (foreground: boolean) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_is_foreground') return new Promise((done) => { resolve = done; });
      if (command === 'capture_crop') return new ArrayBuffer(0);
      if (command === 'capture_export') return false;
    });
    const main = view.container.querySelector('main')!;
    Object.assign(main, { setPointerCapture: vi.fn(), hasPointerCapture: () => false });
    fireEvent.pointerDown(main, { clientX: 10, clientY: 10, button: 0 });
    fireEvent.pointerMove(main, { clientX: 200, clientY: 100 });
    fireEvent.pointerUp(main, { clientX: 200, clientY: 100, button: 0 });
    fireEvent.blur(window);
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(resolve).toBeTypeOf('function');
    await act(async () => { fireEvent.click(view.getByLabelText('保存 PNG')); });
    expect(invoke).toHaveBeenCalledWith('capture_export', expect.any(Uint8Array), expect.anything());
    expect(view.queryByRole('status')).toBeNull();
    await act(async () => { resolve(false); });
    expect(invoke).not.toHaveBeenCalledWith('capture_cancel');
    expect(view.queryByRole('status')).toBeNull();
    view.unmount();
  });

  it('cleans up delayed blur verification on unmount', async () => {
    const view = await shown();
    fireEvent.blur(window); view.unmount();
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(invoke).not.toHaveBeenCalledWith('capture_is_foreground');
    expect(invoke).not.toHaveBeenCalledWith('capture_cancel');
  });
});

describe('capture keyboard lifecycle', () => {
  const rect = { x: 20, y: 20, width: 200, height: 100 };
  beforeEach(() => {
    vi.stubGlobal('PointerEvent', MouseEvent);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.mocked(decodePng).mockResolvedValueOnce(new Image());
  });
  async function select(view: ReturnType<typeof render>): Promise<void> {
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    const main = view.container.querySelector('main')!;
    Object.assign(main, { setPointerCapture: vi.fn(), hasPointerCapture: () => false });
    const at = { clientX: 50 * window.innerWidth / meta.screen.width, clientY: 50 * window.innerHeight / meta.screen.height, button: 0 };
    fireEvent.pointerDown(main, at); fireEvent.pointerUp(main, at);
    await waitFor(() => expect(view.getByLabelText('保存 PNG')).toBeInTheDocument());
  }

  it('cancels before decode completes without showing the old session afterwards', async () => {
    let resolve!: (image: HTMLImageElement) => void;
    vi.mocked(decodePng).mockReset().mockImplementation(() => new Promise((done) => { resolve = done; }));
    const view = render(<CaptureApp />);
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    fireEvent.keyDown(document.body, { key: 'Escape' });
    await act(async () => { resolve(new Image()); });
    expect(invoke).toHaveBeenCalledWith('capture_cancel');
    expect(invoke).not.toHaveBeenCalledWith('capture_show');
    view.unmount();
  });

  it('cancels busy cropping once and prevents a late crop from exporting or unlocking the session', async () => {
    let resolve!: (bytes: ArrayBuffer) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return { ...meta, pointer: { x: 50, y: 50 }, targets: [{ rect, label: 'window', controls: [] }] };
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_crop') return new Promise((done) => { resolve = done; });
    });
    const view = render(<CaptureApp />); await select(view);
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    fireEvent.keyDown(window, { key: 'Escape' }); fireEvent.keyDown(window, { key: 'Escape', repeat: true });
    await act(async () => { resolve(new ArrayBuffer(0)); });
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'capture_cancel')).toHaveLength(1);
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'capture_export')).toBe(false);
    expect(view.getByRole('status')).toBeInTheDocument();
    view.unmount();
  });

  it('keeps cancellation active when an in-flight export returns after Escape', async () => {
    let resolve!: (finished: boolean) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return { ...meta, pointer: { x: 50, y: 50 }, targets: [{ rect, label: 'window', controls: [] }] };
      if (command === 'capture_image' || command === 'capture_crop') return new ArrayBuffer(0);
      if (command === 'capture_export') return new Promise((done) => { resolve = done; });
    });
    const view = render(<CaptureApp />); await select(view);
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    fireEvent.keyDown(window, { key: 'Escape' });
    await act(async () => { resolve(false); });
    expect(invoke).toHaveBeenCalledWith('capture_cancel');
    expect(view.getByRole('status')).toBeInTheDocument();
    expect(view.container.querySelector('.capture-magnifier')).toBeNull();
    view.unmount();
  });

  it('allows Escape retries after a failed cancel even when an input stops bubbling', async () => {
    let attempts = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return { ...meta, pointer: { x: 50, y: 50 }, targets: [{ rect, label: 'window', controls: [] }] };
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_cancel' && ++attempts === 1) throw new Error('cancel failed');
    });
    const view = render(<CaptureApp />); await select(view);
    const input = view.getByLabelText('线宽');
    input.addEventListener('keydown', (event) => event.stopPropagation());
    input.focus(); fireEvent.keyDown(input, { key: 'Escape' });
    await view.findByRole('alert');
    fireEvent.keyDown(input, { key: 'Escape' });
    await waitFor(() => expect(attempts).toBe(2));
    view.unmount();
  });

  it('applies Tab pressed before provider results and traps focus even when a descendant stops bubbling', async () => {
    let resolve!: (response: CaptureHitTest) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return { ...meta, pointer: { x: 50, y: 50 }, targets: [] };
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_candidates') return new Promise((done) => { resolve = done; });
    });
    const view = render(<CaptureApp />);
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    const main = view.container.querySelector('main')!;
    main.addEventListener('keydown', (event) => event.stopPropagation());
    expect(fireEvent.keyDown(main, { key: 'Tab' })).toBe(false);
    const parent = { rect: { x: 0, y: 0, width: 1000, height: 800 }, label: 'parent' };
    await act(async () => { resolve({ candidates: [{ rect, label: 'control' }, parent], pending: false }); });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('1000 × 800');
    fireEvent.keyDown(main, { key: 'Tab', shiftKey: true });
    expect(view.container.querySelector('.capture-size')).toHaveTextContent('200 × 100');
    view.unmount();
  });

  it('discards pending click selection after Escape', async () => {
    let resolve!: (response: CaptureHitTest) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_session') return { ...meta, targets: [] };
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_candidates') return new Promise((done) => { resolve = done; });
    });
    const view = render(<CaptureApp />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_show'));
    const main = view.container.querySelector('main')!;
    Object.assign(main, { setPointerCapture: vi.fn(), hasPointerCapture: () => false });
    fireEvent.pointerDown(main, { clientX: 50, clientY: 50, button: 0 });
    fireEvent.pointerUp(main, { clientX: 50, clientY: 50, button: 0 });
    fireEvent.keyDown(window, { key: 'Escape' });
    await act(async () => { resolve({ candidates: [{ rect, label: 'control' }], pending: false }); });
    expect(view.queryByLabelText('保存 PNG')).toBeNull();
    view.unmount();
  });
});
