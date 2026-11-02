import { StrictMode } from 'react';
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { RegionRecording } from './RegionRecording';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const layout = { selection: { x: 50, y: 80, width: 800, height: 600 }, toolbar: { x: 50, y: 700, width: 300, height: 48 } };
const screen = { x: -1920, y: 0, width: 1920, height: 1080 };
const stopped = { started: true, elapsedMs: 65000, paused: false, stopped: true, error: null };
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
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    fireEvent.click(view.getByLabelText('停止并保存'));
    await waitFor(() => expect(complete).toBeTypeOf('function'));
    expect(view.getByLabelText('保存 MP4')).toBeDisabled();
    fireEvent.click(view.getByLabelText('保存 MP4'));
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'capture_record_save')).toHaveLength(1);
    fireEvent.click(view.getByLabelText('丢弃录制'));
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
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    fireEvent.click(view.getByLabelText('停止并保存'));
    view.unmount();
    await act(async () => complete(stopped));
    expect(invoke).not.toHaveBeenCalledWith('capture_record_save');
  });
  it('does not save after discarding while stopping', async () => {
    let complete!: (value: typeof stopped) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_record_status') return { ...stopped, stopped: false };
      if (command === 'capture_record_stop') return new Promise((done) => { complete = done; });
    });
    const cancel = vi.fn();
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={cancel} />);
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    fireEvent.click(view.getByLabelText('停止并保存'));
    fireEvent.click(view.getByLabelText('丢弃录制'));
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

  it('pauses and resumes using the native state without restarting the recording', async () => {
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, ...args) => command === 'capture_record_pause'
      ? { ...stopped, stopped: false, paused: (args[0] as { paused: boolean }).paused }
      : implementation(command, ...args));
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={vi.fn()} />);
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    fireEvent.click(view.getByLabelText('暂停录制'));
    await waitFor(() => expect(view.getByLabelText('继续录制')).toBeEnabled());
    expect(view.getByRole('status')).toHaveTextContent('已暂停');
    expect(invoke).toHaveBeenCalledWith('capture_record_pause', { paused: true });
    fireEvent.click(view.getByLabelText('继续录制'));
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    expect(invoke).toHaveBeenCalledWith('capture_record_pause', { paused: false });
    expect(invoke).not.toHaveBeenCalledWith('capture_record_stop');
    view.unmount();
  });
  it('stops with Escape and retains the video without opening a save dialog', async () => {
    const cancel = vi.fn();
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={cancel} />);
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    fireEvent.keyDown(window, { key: 'Escape' });
    fireEvent.keyDown(window, { key: 'Escape', repeat: true });
    await waitFor(() => expect(view.getByLabelText('保存 MP4')).toBeEnabled());
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'capture_record_stop')).toHaveLength(1);
    expect(invoke).not.toHaveBeenCalledWith('capture_record_save');
    expect(cancel).not.toHaveBeenCalled();
    view.unmount();
  });
  it('ignores a stale status result after pausing', async () => {
    let resolveStatus!: (value: typeof stopped) => void;
    let polls = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_record_status' && ++polls === 1) return { ...stopped, stopped: false };
      if (command === 'capture_record_status') return new Promise((done) => { resolveStatus = done; });
      if (command === 'capture_record_pause') return { ...stopped, stopped: false, paused: true };
    });
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={vi.fn()} />);
    await waitFor(() => expect(resolveStatus).toBeTypeOf('function'));
    fireEvent.click(view.getByLabelText('暂停录制'));
    await waitFor(() => expect(view.getByLabelText('继续录制')).toBeEnabled());
    await act(async () => resolveStatus({ ...stopped, stopped: false }));
    expect(view.getByRole('status')).toHaveTextContent('已暂停');
    view.unmount();
  });
  it('leaves pause controls available after a failed pause request', async () => {
    const implementation = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, ...args) => {
      if (command === 'capture_record_pause') throw new Error('pause failed');
      return implementation(command, ...args);
    });
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={vi.fn()} />);
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    fireEvent.click(view.getByLabelText('暂停录制'));
    await waitFor(() => expect(view.getByRole('alert')).toHaveTextContent('pause failed'));
    expect(view.getByLabelText('暂停录制')).toBeEnabled();
    expect(view.getByLabelText('停止并保存')).toBeEnabled();
    view.unmount();
  });
  it('recovers polling after a temporary status failure', async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error('status unavailable')).mockResolvedValue({ ...stopped, stopped: false });
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={vi.fn()} />);
    await waitFor(() => expect(view.getByRole('alert')).toHaveTextContent('status unavailable'));
    await waitFor(() => expect(view.queryByRole('alert')).toBeNull());
    expect(view.getByRole('timer')).toHaveTextContent('01:05');
    view.unmount();
  });

  it('waits for Start and cycles the same control through start, pause, resume, and pause', async () => {
    let state = { ...stopped, started: false, stopped: false, elapsedMs: 0 };
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'capture_record_start') state = { ...state, started: true };
      if (command === 'capture_record_pause') state = { ...state, paused: (args as { paused: boolean }).paused };
      return state;
    });
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={vi.fn()} />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('capture_record_status'));
    const button = view.getByLabelText('开始录制');
    expect(button).toHaveTextContent('开始');
    expect(view.getByRole('timer')).toHaveTextContent('00:00');
    expect(view.getByRole('status')).toHaveTextContent('准备就绪');
    expect(view.getByLabelText('停止并保存')).toBeDisabled();
    expect(invoke).not.toHaveBeenCalledWith('capture_record_start');
    expect(invoke).not.toHaveBeenCalledWith('capture_record_pause', expect.anything());
    fireEvent.click(button);
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    expect(view.getByLabelText('暂停录制')).toBe(button);
    expect(button).toHaveTextContent('暂停');
    fireEvent.click(button);
    await waitFor(() => expect(view.getByLabelText('继续录制')).toBeEnabled());
    expect(view.getByLabelText('继续录制')).toBe(button);
    expect(button).toHaveTextContent('继续');
    fireEvent.click(button);
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    fireEvent.click(button);
    await waitFor(() => expect(view.getByLabelText('继续录制')).toBeEnabled());
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'capture_record_start')).toHaveLength(1);
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'capture_record_pause').map(([, args]) => args)).toEqual([{ paused: true }, { paused: false }, { paused: true }]);
    view.unmount();
  });
  it('blocks duplicate Start requests and retries a failed start without changing controls', async () => {
    let reject!: (reason: Error) => void;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'capture_record_status') return { ...stopped, started: false, stopped: false, elapsedMs: 0 };
      if (command === 'capture_record_start') return new Promise((_, fail) => { reject = fail; });
    });
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={vi.fn()} />);
    fireEvent.click(view.getByLabelText('开始录制'));
    expect(view.getByLabelText('开始录制')).toBeDisabled();
    fireEvent.click(view.getByLabelText('开始录制'));
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'capture_record_start')).toHaveLength(1);
    await act(async () => reject(new Error('encoder unavailable')));
    expect(view.getByRole('alert')).toHaveTextContent('encoder unavailable');
    expect(view.getByLabelText('开始录制')).toBeEnabled();
    expect(view.getByLabelText('停止并保存')).toBeDisabled();
    vi.mocked(invoke).mockResolvedValue({ ...stopped, stopped: false });
    fireEvent.click(view.getByLabelText('开始录制'));
    await waitFor(() => expect(view.getByLabelText('暂停录制')).toBeEnabled());
    view.unmount();
  });
  it('exits preparation with Escape without stopping or saving an empty video', async () => {
    vi.mocked(invoke).mockResolvedValue({ ...stopped, started: false, stopped: false, elapsedMs: 0 });
    const cancel = vi.fn();
    const view = render(<RegionRecording layout={layout} screen={screen} onCancel={cancel} />);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(cancel).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalledWith('capture_record_start');
    expect(invoke).not.toHaveBeenCalledWith('capture_record_stop');
    expect(invoke).not.toHaveBeenCalledWith('capture_record_save');
    view.unmount();
  });

});
