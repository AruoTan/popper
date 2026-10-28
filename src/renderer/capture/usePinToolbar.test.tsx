import { StrictMode, useEffect } from 'react';
import { act, render, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { usePinToolbar } from './usePinToolbar';
import type { PinToolbarState } from '../../shared/capture';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
const state: PinToolbarState = { tool: 'select', color: '#ff4545', lineWidth: 3, enabled: true, busy: false, canUndo: false, canRedo: false, canDelete: false };
beforeEach(() => { vi.clearAllMocks(); vi.mocked(listen).mockResolvedValue(vi.fn()); vi.mocked(invoke).mockResolvedValue(undefined); });

describe('pin toolbar state channel', () => {
  it('coalesces waiting snapshots and resumes after a failed publication', async () => {
    let finish!: () => void;
    vi.mocked(invoke).mockImplementationOnce(() => new Promise((resolve) => { finish = () => resolve(undefined); }));
    const error = vi.fn(), hook = renderHook(() => usePinToolbar(vi.fn(), error));
    act(() => {
      hook.result.current(state);
      hook.result.current({ ...state, tool: 'rectangle' });
      hook.result.current({ ...state, tool: 'pen' });
    });
    expect(invoke).toHaveBeenCalledTimes(1);
    await act(async () => finish());
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenLastCalledWith('capture_pin_toolbar_publish', { snapshot: { ...state, tool: 'pen' } });
    vi.mocked(invoke).mockRejectedValueOnce(new Error('publish unavailable'));
    await act(async () => hook.result.current(state));
    expect(error).toHaveBeenCalledWith(expect.objectContaining({ message: 'publish unavailable' }));
    await act(async () => hook.result.current({ ...state, lineWidth: 5 }));
    expect(invoke).toHaveBeenLastCalledWith('capture_pin_toolbar_publish', { snapshot: { ...state, lineWidth: 5 } });
    hook.unmount();
  });

  it('does not publish queued state or report a late failure after unmount', async () => {
    let fail!: (error: Error) => void;
    vi.mocked(invoke).mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
    const error = vi.fn(), hook = renderHook(() => usePinToolbar(vi.fn(), error));
    act(() => { hook.result.current(state); hook.result.current({ ...state, tool: 'pen' }); });
    hook.unmount();
    await act(async () => fail(new Error('late response')));
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(error).not.toHaveBeenCalled();
  });

  it('keeps the newest publication after StrictMode replays its effects', async () => {
    let finish!: () => void;
    vi.mocked(invoke).mockImplementationOnce(() => new Promise((resolve) => { finish = () => resolve(undefined); }));
    function Host() {
      const publish = usePinToolbar(vi.fn(), vi.fn());
      useEffect(() => publish(state), [publish]);
      return null;
    }
    const view = render(<StrictMode><Host /></StrictMode>);
    await act(async () => finish());
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    view.unmount();
  });
});
