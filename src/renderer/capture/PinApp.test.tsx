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
