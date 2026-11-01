import { StrictMode } from 'react';
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { RegionRecording } from './RegionRecording';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const layout = { selection: { x: 50, y: 80, width: 800, height: 600 }, toolbar: { x: 50, y: 700, width: 300, height: 48 } };
const screen = { x: -1920, y: 0, width: 1920, height: 1080 };
const stopped = { elapsedMs: 65000, stopped: true, error: null };
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'capture_record_status') return { ...stopped, stopped: false };
    if (command === 'capture_record_stop') return stopped;
    if (command === 'capture_record_save') return false;
  });
});
describe('region recording controls', () => {
  it('shows elapsed time and retains a stopped video when the save dialog is cancelled', async () => {
    const cancel = vi.fn();
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={cancel} />);
    await waitFor(() => expect(view.getByRole('timer')).toHaveTextContent('01:05'));
    fireEvent.click(view.getByLabelText('停止并保存'));
    await waitFor(() => expect(view.getByLabelText('保存 MP4')).toBeEnabled());
    expect(invoke).toHaveBeenCalledWith('capture_record_stop');
    expect(invoke).toHaveBeenCalledWith('capture_record_save');
    expect(cancel).not.toHaveBeenCalled();
    fireEvent.click(view.getByLabelText('保存 MP4'));
    await waitFor(() => expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'capture_record_save')).toHaveLength(2));
    view.unmount();
  });
  it('blocks duplicate output and lets the user cancel during a pending save', async () => {
    let complete!: (value: boolean) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_record_status') return { ...stopped, stopped: false };
      if (command === 'capture_record_stop') return stopped;
      if (command === 'capture_record_save') return new Promise((done) => { complete = done; });
    });
    const cancel = vi.fn();
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={cancel} />);
    fireEvent.click(view.getByLabelText('停止并保存'));
    await waitFor(() => expect(complete).toBeTypeOf('function'));
    expect(view.getByLabelText('保存 MP4')).toBeDisabled();
    fireEvent.click(view.getByLabelText('保存 MP4'));
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'capture_record_save')).toHaveLength(1);
    fireEvent.click(view.getByLabelText('取消录屏'));
    expect(cancel).toHaveBeenCalledOnce();
    view.unmount();
    await act(async () => complete(false));
  });
  it('allows retry after a disk error and displays automatic capture failure', async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_record_status') return { ...stopped, error: '显示器布局已变化' };
      if (command === 'capture_record_stop') return stopped;
      if (command === 'capture_record_save') throw new Error('disk full');
    });
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={vi.fn()} />);
    await waitFor(() => expect(view.getByRole('alert')).toHaveTextContent('显示器布局已变化'));
    fireEvent.click(view.getByLabelText('保存 MP4'));
    await waitFor(() => expect(view.getByRole('alert')).toHaveTextContent('disk full'));
    expect(view.getByLabelText('保存 MP4')).toBeEnabled();
    view.unmount();
  });
  it('does not open a save dialog after unmount while stopping', async () => {
    let complete!: (value: typeof stopped) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_record_status') return { ...stopped, stopped: false };
      if (command === 'capture_record_stop') return new Promise((done) => { complete = done; });
    });
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={vi.fn()} />);
    fireEvent.click(view.getByLabelText('停止并保存'));
    view.unmount();
    await act(async () => complete(stopped));
    expect(invoke).not.toHaveBeenCalledWith('capture_record_save');
  });
  it.each(['button', 'Escape'])('does not save after cancellation by %s while stopping', async (method) => {
    let complete!: (value: typeof stopped) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_record_status') return { ...stopped, stopped: false };
      if (command === 'capture_record_stop') return new Promise((done) => { complete = done; });
    });
    const cancel = vi.fn();
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={cancel} />);
    fireEvent.click(view.getByLabelText('停止并保存'));
    if (method === 'button') fireEvent.click(view.getByLabelText('取消录屏'));
    else fireEvent.keyDown(window, { key: 'Escape' });
    await act(async () => complete(stopped));
    expect(cancel).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalledWith('capture_record_save');
    view.unmount();
  });

  it('ignores an obsolete status response when StrictMode replays the effect', async () => {
    let resolveOld!: (value: typeof stopped) => void;
    vi.mocked(invoke).mockImplementationOnce(() => new Promise((done) => { resolveOld = done; }))
      .mockResolvedValue({ ...stopped, elapsedMs: 2000 });
    const view = render(<StrictMode><RegionRecording layout={layout} screen={screen} onCancel={vi.fn()} /></StrictMode>);
    await waitFor(() => expect(view.getByLabelText('保存 MP4')).toBeEnabled());
    await act(async () => resolveOld({ ...stopped, stopped: false }));
    expect(view.getByRole('timer')).toHaveTextContent('00:02');
    expect(view.getByLabelText('保存 MP4')).toBeEnabled();
    view.unmount();
  });

});
