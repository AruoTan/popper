import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { decodePng } from './drawing';
import { PinApp } from './PinApp';
import type { PinnedImageState } from '../../shared/capture';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('./drawing', () => ({ decodePng: vi.fn() }));
const state: PinnedImageState = { id: 'pin', width: 400, height: 240, zoom: 1, opacity: 1, clickThrough: false, shadowInset: 24 };
beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal('URL', { createObjectURL: vi.fn(() => 'blob:pin-image'), revokeObjectURL: vi.fn() });
  vi.mocked(decodePng).mockResolvedValue({ naturalWidth: state.width, naturalHeight: state.height } as HTMLImageElement);
  vi.mocked(listen).mockResolvedValue(vi.fn());
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === 'capture_pin_state') return state;
    if (command === 'capture_image') return new ArrayBuffer(0);
    if (command === 'capture_pin_update') return { ...state, ...(args as { update: object }).update };
    if (command === 'capture_pin_drag') return { x: 124, y: 84 };
  });
});

describe('pinned image dragging', () => {
  async function imageTarget() {
    const view = render(<PinApp />);
    const image = await view.findByAltText('浮动图片');
    const main = image.closest('main')!;
    Object.assign(main, { setPointerCapture: vi.fn(), hasPointerCapture: () => true, releasePointerCapture: vi.fn() });
    return { view, image, main };
  }
  const moves = () => vi.mocked(invoke).mock.calls.filter(([command, args]) => command === 'capture_pin_drag' && (args as { anchor: unknown }).anchor !== null);

  it('keeps the native physical anchor when dragging above the screen and across negative coordinates', async () => {
    const { view, main } = await imageTarget();
    fireEvent.pointerDown(main, { button: 0, buttons: 1, pointerId: 7, clientX: 100, clientY: 60 });
    await act(async () => {});
    fireEvent.pointerMove(main, { buttons: 1, pointerId: 7, clientX: 103, clientY: 60 });
    expect(moves()).toHaveLength(0);
    fireEvent.pointerMove(main, { buttons: 1, pointerId: 7, clientX: -120, clientY: -20, screenX: -1720, screenY: -30 });
    await waitFor(() => expect(moves()).toHaveLength(1));
    expect(moves()[0]).toEqual(['capture_pin_drag', { anchor: { x: 124, y: 84 } }]);
    expect(main.setPointerCapture).toHaveBeenCalledWith(7);
    fireEvent.pointerUp(main, { button: 0, buttons: 0, pointerId: 7 });
    fireEvent.pointerMove(main, { buttons: 1, pointerId: 7, clientX: -200, clientY: -200 });
    expect(moves()).toHaveLength(1);
    expect(main.releasePointerCapture).toHaveBeenCalledWith(7);
    view.unmount();
  });

  it('coalesces moves while the native command is busy and samples again after it completes', async () => {
    const original = vi.mocked(invoke).getMockImplementation()!;
    let finish!: (point: { x: number; y: number }) => void;
    vi.mocked(invoke).mockImplementation((command, args) => {
      if (command === 'capture_pin_drag' && (args as { anchor: unknown }).anchor !== null) return new Promise((resolve) => { finish = resolve; });
      return original(command, args);
    });
    const { view, main } = await imageTarget();
    fireEvent.pointerDown(main, { button: 0, buttons: 1, pointerId: 1, clientX: 40, clientY: 40 });
    await act(async () => {});
    for (let index = 0; index < 20; index++) fireEvent.pointerMove(main, { buttons: 1, pointerId: 1, clientX: 60 + index, clientY: -50 });
    expect(moves()).toHaveLength(1);
    await act(async () => { finish({ x: 124, y: 84 }); });
    expect(moves()).toHaveLength(2);
    fireEvent.pointerUp(main, { button: 0, buttons: 0, pointerId: 1 });
    await act(async () => { finish({ x: 124, y: 84 }); });
    expect(moves()).toHaveLength(2);
    view.unmount();
  });

  it.each(['pointerUp', 'pointerCancel', 'lostPointerCapture', 'blur', 'close', 'unmount'])('ignores a late anchor after %s', async (reason) => {
    const original = vi.mocked(invoke).getMockImplementation()!;
    let finish!: (point: { x: number; y: number }) => void;
    vi.mocked(invoke).mockImplementation((command, args) => command === 'capture_pin_drag'
      ? new Promise((resolve) => { finish = resolve; }) : original(command, args));
    const { view, main } = await imageTarget();
    fireEvent.pointerDown(main, { button: 0, buttons: 1, pointerId: 1, clientX: 40, clientY: 40 });
    fireEvent.pointerMove(main, { buttons: 1, pointerId: 1, clientX: 100, clientY: -50 });
    if (reason === 'blur') fireEvent(window, new Event('blur'));
    else if (reason === 'close') fireEvent.keyDown(window, { key: 'Escape' });
    else if (reason === 'unmount') view.unmount();
    else fireEvent[reason as 'pointerUp' | 'pointerCancel' | 'lostPointerCapture'](main, { pointerId: 1 });
    await act(async () => { finish({ x: 124, y: 84 }); });
    expect(moves()).toHaveLength(0);
    if (reason !== 'unmount') view.unmount();
  });

  it('stops quietly when the native button has already been released', async () => {
    const original = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((command, args) => command === 'capture_pin_drag' ? Promise.resolve(null) : original(command, args));
    const { view, main } = await imageTarget();
    fireEvent.pointerDown(main, { button: 0, buttons: 1, pointerId: 1 });
    await act(async () => {});
    fireEvent.pointerMove(main, { buttons: 1, pointerId: 1, clientX: 100, clientY: -50 });
    expect(moves()).toHaveLength(0);
    expect(view.queryByRole('alert')).toBeNull();
    view.unmount();
  });

  it('reports native errors and allows a subsequent drag', async () => {
    const { view, main } = await imageTarget();
    vi.mocked(invoke).mockRejectedValueOnce(new Error('move failed'));
    fireEvent.pointerDown(main, { button: 0, buttons: 1, pointerId: 1 });
    expect(await view.findByRole('alert')).toHaveTextContent('move failed');
    fireEvent.pointerDown(main, { button: 0, buttons: 1, pointerId: 2 });
    await act(async () => {});
    fireEvent.pointerMove(main, { buttons: 1, pointerId: 2, clientX: 100, clientY: -50 });
    await waitFor(() => expect(moves()).toHaveLength(1));
    view.unmount();
  });
});
afterEach(() => vi.unstubAllGlobals());

describe('pinned image presentation', () => {
  it('shows the decoded image before starting its shadow and replays the shadow on restoration', async () => {
    let shown!: () => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_pin_state') return state;
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_show') return new Promise<void>((resolve) => { shown = resolve; });
    });
    const view = render(<PinApp />);
    const image = await view.findByAltText('浮动图片');
    expect(invoke).not.toHaveBeenCalledWith('capture_show');
    expect(view.container.querySelector('.capture-pin-shadow')).toBeNull();
    fireEvent.load(image);
    expect(invoke).toHaveBeenCalledWith('capture_show');
    expect(view.container.querySelector('.capture-pin-shadow')).toBeNull();
    await act(async () => { shown(); });
    const shadow = view.container.querySelector('.capture-pin-shadow');
    expect(shadow).not.toBeNull();
    const callback = vi.mocked(listen).mock.calls.find(([event]) => event === 'popper:pin-shown')![1];
    act(() => callback({ event: 'popper:pin-shown', id: 1, payload: undefined }));
    expect(view.container.querySelector('.capture-pin-shadow')).not.toBe(shadow);
    view.unmount();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:pin-image');
  });

  it('anchors wheel zoom to the image contents rather than the shadow frame', async () => {
    const view = render(<PinApp />);
    const image = await view.findByAltText('浮动图片');
    const main = image.closest('main')!;
    vi.spyOn(main, 'getBoundingClientRect').mockReturnValue({ left: 24, top: 24, width: 400, height: 240 } as DOMRect);
    fireEvent.wheel(main, { deltaY: -120, clientX: 124, clientY: 204 });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_pin_update', { update: { zoom: 1.1, opacity: 1, clickThrough: false, anchorX: 0.25, anchorY: 0.75 } }));
    view.unmount();
  });

  it('discards a late show response after the pin has been destroyed', async () => {
    let shown!: () => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_pin_state') return state;
      if (command === 'capture_image') return new ArrayBuffer(0);
      if (command === 'capture_show') return new Promise<void>((resolve) => { shown = resolve; });
    });
    const view = render(<PinApp />);
    fireEvent.load(await view.findByAltText('浮动图片'));
    view.unmount();
    await act(async () => { shown(); });
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:pin-image');
  });
});

describe('pinned image keyboard focus', () => {
  it('handles Escape outside the image and accepts another close after restoration', async () => {
    const view = render(<PinApp />);
    await view.findByAltText('浮动图片');
    fireEvent.keyDown(document.body, { key: 'Escape' });
    fireEvent.keyDown(window, { key: 'Escape', repeat: true });
    expect(invoke).toHaveBeenCalledWith('capture_pin_close', { destroy: false });
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'capture_pin_close')).toHaveLength(1);
    const callback = vi.mocked(listen).mock.calls.find(([event]) => event === 'popper:pin-shown')![1];
    act(() => callback({ event: 'popper:pin-shown', id: 1, payload: undefined }));
    fireEvent.keyDown(window, { key: 'Escape', shiftKey: true });
    expect(invoke).toHaveBeenLastCalledWith('capture_pin_close', { destroy: true });
    view.unmount();
  });
  it('restores DOM focus on native window activation and intercepts stopped key events', async () => {
    const view = render(<PinApp />);
    const image = await view.findByAltText('浮动图片');
    const main = image.closest('main')!;
    fireEvent(window, new Event('focus'));
    expect(document.activeElement).toBe(main);
    main.addEventListener('keydown', (event) => event.stopPropagation());
    fireEvent.keyDown(main, { key: 'Escape' });
    expect(invoke).toHaveBeenCalledWith('capture_pin_close', { destroy: false });
    view.unmount();
  });
});
