import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CaptureCandidate, CaptureHitTest, CaptureTarget } from '../../shared/capture';
import { createCaptureHover } from './hover';

const windowTarget: CaptureTarget = { rect: { x: 0, y: 0, width: 1000, height: 800 }, label: 'window', controls: [] };
const control: CaptureCandidate = { rect: { x: 10, y: 10, width: 100, height: 80 }, label: 'control' };
const response = (candidates: CaptureCandidate[] = [control], pending = false): CaptureHitTest => ({ candidates, pending });
const deferred = (): { promise: Promise<CaptureHitTest>; resolve: (value: CaptureHitTest) => void } => {
  let resolve!: (value: CaptureHitTest) => void;
  const promise = new Promise<CaptureHitTest>((done) => { resolve = done; });
  return { promise, resolve };
};
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('capture hover feedback', () => {
  it('snaps to captured native geometry immediately and coalesces to the latest point per frame', async () => {
    const request = vi.fn(async () => response()), show = vi.fn();
    const hover = createCaptureHover({ targets: [{ ...windowTarget, controls: [control] }], active: () => true, request, show });
    hover.update({ x: 20, y: 20 });
    expect(show.mock.lastCall?.[1].pending).toBe(true);
    expect(show.mock.lastCall?.[0][0]).toEqual(control);
    expect(request).not.toHaveBeenCalled();
    hover.update({ x: 21, y: 22 }); hover.update({ x: 24, y: 25 });
    await vi.advanceTimersByTimeAsync(16);
    expect(request).toHaveBeenCalledExactlyOnceWith({ x: 24, y: 25 });
    expect(show.mock.lastCall?.[1].pending).toBe(false);
    hover.destroy();
  });
  it('uses a valid in-flight result while the pointer is moving and promptly refines the latest point', async () => {
    const first = deferred(), second = deferred(), show = vi.fn();
    const request = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const hover = createCaptureHover({ targets: [windowTarget], active: () => true, request, show });
    hover.update({ x: 20, y: 20 }); await vi.advanceTimersByTimeAsync(16);
    hover.update({ x: 22, y: 24 }); hover.update({ x: 30, y: 30 });
    expect(request).toHaveBeenCalledTimes(1);
    first.resolve(response()); await vi.advanceTimersByTimeAsync(0);
    expect(show.mock.lastCall?.[0][0]).toEqual(control);
    await vi.advanceTimersByTimeAsync(16);
    expect(request).toHaveBeenLastCalledWith({ x: 30, y: 30 });
    hover.destroy(); second.resolve(response());
  });
  it('reuses a cached child immediately when returning to it and discards out-of-point bounds', async () => {
    const request = vi.fn(async () => response()), show = vi.fn();
    const hover = createCaptureHover({ targets: [windowTarget], active: () => true, request, show });
    hover.update({ x: 20, y: 20 }); await vi.advanceTimersByTimeAsync(16);
    hover.update({ x: 500, y: 500 });
    expect(show.mock.lastCall?.[0][0]?.label).toBe('window');
    hover.update({ x: 20, y: 20 });
    expect(show.mock.lastCall?.[0][0]).toEqual(control);
    hover.destroy();
  });
  it('never snaps through the topmost captured window to a cached control behind it', async () => {
    const front: CaptureTarget = { rect: { x: 0, y: 0, width: 30, height: 30 }, label: 'front', controls: [] };
    const show = vi.fn(), request = vi.fn(async () => response());
    const hover = createCaptureHover({ targets: [front, windowTarget], active: () => true, request, show });
    hover.update({ x: 40, y: 40 }); await vi.advanceTimersByTimeAsync(16);
    hover.update({ x: 20, y: 20 });
    expect(show.mock.lastCall?.[0].map((item: CaptureCandidate) => item.label)).toEqual(['front']);
    hover.destroy();
  });
  it('refines a stationary timeout with bounded retries while preserving immediate window feedback', async () => {
    const show = vi.fn(), request = vi.fn(async () => response([], true));
    const hover = createCaptureHover({ targets: [windowTarget], active: () => true, request, show });
    hover.update({ x: 20, y: 20 }); await vi.runAllTimersAsync();
    expect(request).toHaveBeenCalledTimes(3);
    expect(show.mock.lastCall?.[1].pending).toBe(false);
    expect(show.mock.lastCall?.[0][0]?.label).toBe('window');
    hover.destroy();
  });
  it('stops retrying as soon as the warmed provider returns a result', async () => {
    const request = vi.fn().mockResolvedValueOnce(response([], true)).mockResolvedValueOnce(response());
    const show = vi.fn(), hover = createCaptureHover({ targets: [windowTarget], active: () => true, request, show });
    hover.update({ x: 20, y: 20 }); await vi.runAllTimersAsync();
    expect(request).toHaveBeenCalledTimes(2);
    expect(show.mock.lastCall?.[0][0]).toEqual(control);
    hover.destroy();
  });
  it.each(['selection', 'destroy'] as const)('ignores responses after %s', async (reason) => {
    let active = true;
    const result = deferred(), show = vi.fn(), request = vi.fn(() => result.promise);
    const hover = createCaptureHover({ targets: [windowTarget], active: () => active, request, show });
    hover.update({ x: 20, y: 20 }); await vi.advanceTimersByTimeAsync(16);
    show.mockClear();
    if (reason === 'selection') active = false; else hover.destroy();
    result.resolve(response()); await vi.runAllTimersAsync();
    expect(show).not.toHaveBeenCalled(); expect(request).toHaveBeenCalledTimes(1);
    hover.destroy();
  });
  it('retains native targets when the provider fails and does not query unchanged pointer events', async () => {
    const request = vi.fn(async () => { throw new Error('provider unavailable'); }), show = vi.fn();
    const hover = createCaptureHover({ targets: [windowTarget], active: () => true, request, show });
    hover.update({ x: 20, y: 20 }); await vi.runAllTimersAsync(); hover.update({ x: 20, y: 20 });
    await vi.runAllTimersAsync();
    expect(request).toHaveBeenCalledTimes(1); expect(show.mock.lastCall?.[0][0]?.label).toBe('window');
    expect(show.mock.lastCall?.[1].pending).toBe(false);
    hover.destroy();
  });
});
