import { act, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import type { ScrollCaptureProgress } from '../../shared/capture';
import { ScrollCapture } from './ScrollCapture';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const initial: ScrollCaptureProgress = { width: 800, height: 600, frames: 1, status: 'unchanged' };
beforeEach(() => { vi.useFakeTimers(); vi.mocked(invoke).mockReset().mockResolvedValue(initial); });
afterEach(() => vi.useRealTimers());
const advance = async (ms = 180): Promise<void> => { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); };
describe('scroll capture controller', () => {
  it('waits for initialization and never overlaps image requests', async () => {
    const view = render(<ScrollCapture initial={null} onFinish={vi.fn()} onCancel={vi.fn()} />);
    await advance(1000); expect(invoke).not.toHaveBeenCalled();
    expect(view.getByText('完成')).toBeDisabled();
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    view.rerender(<ScrollCapture initial={initial} onFinish={vi.fn()} onCancel={vi.fn()} />);
    await advance(2000); expect(invoke).toHaveBeenCalledTimes(1);
    await act(async () => { resolve({ ...initial, height: 920, frames: 2, status: 'appended' }); });
    expect(view.getByText('800 × 920 · 2 帧')).toBeInTheDocument();
    await advance(); expect(invoke).toHaveBeenCalledTimes(2); view.unmount();
  });
  it('pauses automatic scrolling after three unchanged captures', async () => {
    const view = render(<ScrollCapture initial={initial} onFinish={vi.fn()} onCancel={vi.fn()} />);
    fireEvent.click(view.getByText('自动向下滚动')); await advance(540);
    expect(invoke).toHaveBeenCalledWith('capture_scroll_step', { automatic: true });
    expect(view.getByText('自动向下滚动')).toHaveAttribute('aria-pressed', 'false');
    expect(view.getByRole('status')).toHaveTextContent('画面未继续滚动');
    await advance(); expect(invoke).toHaveBeenLastCalledWith('capture_scroll_step', { automatic: false }); view.unmount();
  });
  it.each(['unmatched', 'limit'] as const)('keeps previous dimensions and pauses automation on %s', async (status) => {
    vi.mocked(invoke).mockResolvedValue({ ...initial, status });
    const view = render(<ScrollCapture initial={initial} onFinish={vi.fn()} onCancel={vi.fn()} />);
    fireEvent.click(view.getByText('自动向下滚动')); await advance();
    expect(view.getByText('自动向下滚动')).toHaveAttribute('aria-pressed', 'false');
    expect(view.getByText('800 × 600 · 1 帧')).toBeInTheDocument();
    expect(view.getByText('完成')).toBeEnabled();
    if (status === 'limit') { await advance(2000); expect(invoke).toHaveBeenCalledTimes(1); }
    view.unmount();
  });
  it('keeps automation active while tracking previously captured regions', async () => {
    vi.mocked(invoke).mockResolvedValue({ ...initial,status:'tracked',frames:8 });
    const view=render(<ScrollCapture initial={initial} onFinish={vi.fn()} onCancel={vi.fn()} />);
    fireEvent.click(view.getByText('自动向下滚动'));await advance(720);
    expect(view.getByText('暂停自动滚动')).toHaveAttribute('aria-pressed','true');
    expect(view.getByRole('status')).toHaveTextContent('已定位到捕捉过的区域');
    expect(invoke).toHaveBeenLastCalledWith('capture_scroll_step',{automatic:true});view.unmount();
  });
  it('waits for the last capture before finishing and stops polling', async () => {
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    const onFinish = vi.fn().mockResolvedValue(true);
    const view = render(<ScrollCapture initial={initial} onFinish={onFinish} onCancel={vi.fn()} />);
    await advance(); fireEvent.click(view.getByText('完成'));
    expect(onFinish).not.toHaveBeenCalled();
    await act(async () => { resolve(initial); }); expect(onFinish).toHaveBeenCalledTimes(1);
    await advance(2000); expect(invoke).toHaveBeenCalledTimes(1); view.unmount();
  });
  it('allows cancellation while a frame request is blocked and discards late results after unmount', async () => {
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    const onCancel = vi.fn(); const onFinish = vi.fn();
    const view = render(<ScrollCapture initial={initial} onFinish={onFinish} onCancel={onCancel} />);
    await advance(); fireEvent.click(view.getByText('取消')); expect(onCancel).toHaveBeenCalledOnce();
    view.unmount(); await act(async () => { resolve(initial); }); await advance(2000);
    expect(invoke).toHaveBeenCalledTimes(1); expect(onFinish).not.toHaveBeenCalled();
  });
  it('preserves the document on capture errors and permits retrying a failed finish', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('window covered'));
    const onFinish = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    const view = render(<ScrollCapture initial={initial} onFinish={onFinish} onCancel={vi.fn()} />);
    fireEvent.click(view.getByText('自动向下滚动')); await advance();
    expect(view.getByRole('status')).toHaveTextContent('window covered');
    expect(view.getByText('800 × 600 · 1 帧')).toBeInTheDocument();
    await act(async () => { fireEvent.click(view.getByText('完成')); });
    expect(view.getByText('重试完成')).toBeEnabled();
    await advance(2000); expect(invoke).toHaveBeenCalledTimes(1);
    await act(async () => { fireEvent.click(view.getByText('重试完成')); }); expect(onFinish).toHaveBeenCalledTimes(2); view.unmount();
  });
  it.each(['cancel', 'unmount'] as const)('does not finalize a queued finish after %s', async (action) => {
    let resolve!: (value: ScrollCaptureProgress) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((done) => { resolve = done; }));
    const onFinish = vi.fn().mockResolvedValue(true), onCancel = vi.fn();
    const view = render(<ScrollCapture initial={initial} onFinish={onFinish} onCancel={onCancel} />);
    await advance(); fireEvent.click(view.getByText('完成'));
    if (action === 'cancel') fireEvent.click(view.getByText('取消')); else view.unmount();
    await act(async () => { resolve(initial); }); await advance(1000);
    expect(onFinish).not.toHaveBeenCalled();
    if (action === 'cancel') { expect(onCancel).toHaveBeenCalledOnce(); view.unmount(); }
  });
});
