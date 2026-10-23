import { render, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { decodePng } from './drawing';
import { CaptureApp } from './CaptureApp';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('./drawing', () => ({ decodePng: vi.fn(), renderScene: vi.fn(), pngBytes: vi.fn() }));
const meta = { id: 'session', screen: { x: -1920, y: 0, width: 1920, height: 1080 }, scale: 1.5, color: '#ff4545', lineWidth: 3 };
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(invoke).mockImplementation(async (command) => command === 'capture_session' ? meta : command === 'capture_image' ? new ArrayBuffer(0) : undefined);
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
