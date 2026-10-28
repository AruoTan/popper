import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { PinToolbarApp } from './PinToolbarApp';
import type { PinToolbarState } from '../../shared/capture';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
const state: PinToolbarState = { tool: 'select', color: '#ff4545', lineWidth: 3, enabled: true, busy: false, canUndo: true, canRedo: false, canDelete: false };
let resize: () => void;
beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal('ResizeObserver', class { constructor(callback: () => void) { resize = callback; } observe() {} disconnect() {} });
  vi.mocked(listen).mockResolvedValue(vi.fn());
  vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_pin_toolbar_state' ? state : undefined);
});
afterEach(() => vi.unstubAllGlobals());

describe('external pin toolbar', () => {
  it('measures its own window and sends ordered actions to its owner without loading an image', async () => {
    const view = render(<PinToolbarApp />);
    await view.findByRole('toolbar', { name: '贴图工具条' });
    expect(invoke).toHaveBeenCalledWith('capture_pin_toolbar_ready', { height: 49 });
    fireEvent.click(view.getByRole('button', { name: '矩形' }));
    fireEvent.change(view.getByLabelText('标注颜色'), { target: { value: '#00ff00' } });
    fireEvent.click(view.getByRole('button', { name: '复制图像' }));
    fireEvent.click(view.getByRole('button', { name: '另存图像' }));
    fireEvent.click(view.getByRole('button', { name: '隐藏工具条' }));
    await waitFor(() => expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'capture_pin_toolbar_action')).toHaveLength(5));
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'capture_pin_toolbar_action').map(([, args]) => args)).toEqual([
      { action: { action: 'tool', tool: 'rectangle' } }, { action: { action: 'color', color: '#00ff00' } },
      { action: { action: 'copy' } }, { action: { action: 'save' } }, { action: { action: 'hide' } },
    ]);
    expect(invoke).not.toHaveBeenCalledWith('capture_image');
    const surface = view.container.firstElementChild!;
    vi.spyOn(surface, 'getBoundingClientRect').mockReturnValue({ height: 80 } as DOMRect);
    act(() => resize());
    expect(invoke).toHaveBeenCalledWith('capture_pin_toolbar_ready', { height: 80 });
    view.unmount();
  });

  it('keeps streamed state newer than an initial asynchronous snapshot', async () => {
    let finish!: (snapshot: PinToolbarState) => void;
    vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_pin_toolbar_state' ? new Promise((resolve) => { finish = resolve; }) : undefined);
    const view = render(<PinToolbarApp />);
    await waitFor(() => expect(finish).toBeTypeOf('function'));
    const callback = vi.mocked(listen).mock.calls[0]![1];
    act(() => callback({ event: 'popper:pin-toolbar-state', id: 1, payload: { ...state, tool: 'pen', enabled: false, busy: true } }));
    await act(async () => finish(state));
    expect(view.getByRole('button', { name: '画笔' })).toHaveAttribute('aria-pressed', 'true');
    expect(view.getByRole('button', { name: '矩形' })).toBeDisabled();
    expect(view.getByRole('button', { name: '复制图像' })).toBeDisabled();
    view.unmount();
  });

  it('keeps Escape and copy/save keyboard actions available in the toolbar window', async () => {
    const view = render(<PinToolbarApp />); await view.findByRole('toolbar');
    fireEvent.keyDown(window, { key: 'c', ctrlKey: true });
    fireEvent.keyDown(window, { key: 's', ctrlKey: true });
    fireEvent.keyDown(window, { key: 'Escape', shiftKey: true });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_pin_toolbar_action', { action: { action: 'close', destroy: true } }));
    expect(invoke).toHaveBeenCalledWith('capture_pin_toolbar_action', { action: { action: 'copy' } });
    expect(invoke).toHaveBeenCalledWith('capture_pin_toolbar_action', { action: { action: 'save' } });
    view.unmount();
  });

  it('reports command errors and accepts the next action', async () => {
    const view = render(<PinToolbarApp />); await view.findByRole('toolbar');
    vi.mocked(invoke).mockRejectedValueOnce(new Error('owner unavailable'));
    fireEvent.click(view.getByRole('button', { name: '复制图像' }));
    expect(await view.findByRole('alert')).toHaveTextContent('owner unavailable');
    fireEvent.click(view.getByRole('button', { name: '另存图像' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_pin_toolbar_action', { action: { action: 'save' } }));
    view.unmount();
  });

  it('disposes listeners that finish after unmount', async () => {
    let finish!: (dispose: () => void) => void;
    vi.mocked(listen).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const view = render(<PinToolbarApp />); view.unmount();
    const dispose = vi.fn(); await act(async () => finish(dispose));
    expect(dispose).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalledWith('capture_pin_toolbar_state');
  });
});
