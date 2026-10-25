import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { decodePng } from './drawing';
import { CaptureApp } from './CaptureApp';
import type { CaptureHitTest } from '../../shared/capture';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('./drawing', () => ({ decodePng: vi.fn(), renderScene: vi.fn(), pngBytes: vi.fn() }));
const meta = { id: 'session', screen: { x: -1920, y: 0, width: 1920, height: 1080 }, scale: 1.5, color: '#ff4545', lineWidth: 3 };
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_session' ? meta : command === 'capture_image' ? new ArrayBuffer(0) : undefined);
});
afterEach(() => vi.unstubAllGlobals());
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
