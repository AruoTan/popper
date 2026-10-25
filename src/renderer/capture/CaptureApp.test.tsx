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
