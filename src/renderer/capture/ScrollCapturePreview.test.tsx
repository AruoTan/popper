import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ScrollCapturePreview } from './ScrollCapturePreview';
import type { PreparedScrollPreview } from './scrollPreview';

const preview = (height: number, y = 0, originY = 0): PreparedScrollPreview => ({ url: `blob:${height}`, originY, progress: {
  width: 800, height, revision: height, status: 'appended', viewport: { x: 0, y, width: 800, height: 200 },
} });
let clock = 0, sequence = 0;
const frames = new Map<number, FrameRequestCallback>();
const frame = (now: number): void => {
  clock = now; const pending = [...frames.values()]; frames.clear();
  act(() => { for (const callback of pending) callback(now); });
};
beforeEach(() => {
  clock = 0; sequence = 0; frames.clear();
  vi.spyOn(performance, 'now').mockImplementation(() => clock);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.set(++sequence, callback); return sequence; });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => { frames.delete(id); });
});
afterEach(() => { vi.unstubAllGlobals(); });
const parts = (view: ReturnType<typeof render>) => ({
  box: view.container.querySelector<HTMLElement>('.capture-scroll-thumbnail')!,
  img: view.getByRole('img'), marker: view.getByLabelText('当前页面位置'),
});

describe('scroll preview motion', () => {
  it('smoothly reveals added rows even while the width stays unchanged', () => {
    const view = render(<ScrollCapturePreview preview={preview(400)} width={200} height={200} />);
    const { box, img, marker } = parts(view);
    expect(box.style.height).toBe('100px'); expect(frames.size).toBe(0);
    view.rerender(<ScrollCapturePreview preview={preview(600, 200)} width={200} height={200} />);
    expect(box.style.height).toBe('100px'); expect(img.style.transform).toBe('translateY(0px) scale(0.25)');
    frame(80);
    expect(parseFloat(box.style.height)).toBeCloseTo(143.75);
    expect(parseFloat(marker.style.top)).toBeCloseTo(43.75);
    expect(marker.style.width).toBe(box.style.width);
    frame(160); expect(box.style.height).toBe('150px'); expect(marker.style.top).toBe('50px'); expect(frames.size).toBe(0);
    view.unmount();
  });
  it('retargets the displayed scale and synchronizes viewport geometry on every frame', () => {
    const view = render(<ScrollCapturePreview preview={preview(1000)} width={200} height={200} />);
    const { box, img, marker } = parts(view);
    view.rerender(<ScrollCapturePreview preview={preview(2000, 400)} width={200} height={200} />);
    expect(box.style.width).toBe('160px'); expect(marker.style.top).toBe('0px');
    frame(80); const currentWidth = box.style.width, currentTransform = img.style.transform;
    expect(parseFloat(currentWidth)).toBeCloseTo(90); expect(parseFloat(marker.style.top)).toBeCloseTo(35);
    view.rerender(<ScrollCapturePreview preview={preview(4000, 1000)} width={200} height={200} />);
    expect(box.style.width).toBe(currentWidth); expect(img.style.transform).toBe(currentTransform); expect(frames.size).toBe(1);
    frame(160); expect(marker.style.width).toBe(box.style.width);
    expect(parseFloat(marker.style.left)).toBeCloseTo((200 - parseFloat(box.style.width)) / 2);
    frame(240); expect(box.style.width).toBe('40px'); expect(marker.style.top).toBe('50px'); expect(frames.size).toBe(0);
    view.rerender(<ScrollCapturePreview preview={preview(4000, 2000)} width={200} height={200} />);
    frame(280); expect(parseFloat(marker.style.top)).toBeGreaterThan(50); expect(parseFloat(marker.style.top)).toBeLessThan(100);
    view.unmount(); expect(frames.size).toBe(0);
  });
  it('anchors existing content before revealing rows added above it', () => {
    const view = render(<ScrollCapturePreview preview={preview(400)} width={200} height={200} />);
    view.rerender(<ScrollCapturePreview preview={preview(600, 0, -200)} width={200} height={200} />);
    const { box, img } = parts(view);
    expect(img.style.transform).toBe('translateY(-50px) scale(0.25)'); expect(box.style.height).toBe('100px');
    frame(80); expect(img.style.transform).toBe('translateY(-6.25px) scale(0.25)');
    view.rerender(<ScrollCapturePreview preview={preview(800, 0, -400)} width={200} height={200} />);
    expect(img.style.transform).toBe('translateY(-56.25px) scale(0.25)');
    frame(240); expect(img.style.transform).toBe('translateY(0px) scale(0.25)'); expect(box.style.height).toBe('200px'); view.unmount();
  });
  it('does not restart an active animation when only the decoded image is replaced', () => {
    const view = render(<ScrollCapturePreview preview={preview(400)} width={200} height={200} />);
    view.rerender(<ScrollCapturePreview preview={preview(600)} width={200} height={200} />);
    frame(80);
    view.rerender(<ScrollCapturePreview preview={{ ...preview(600), url: 'blob:sharper' }} width={200} height={200} />);
    frame(160); expect(parts(view).box.style.height).toBe('150px'); expect(frames.size).toBe(0); view.unmount();
  });
  it('updates immediately when reduced motion is enabled and stops active motion if the preference changes', () => {
    let change!: () => void;
    const media = { matches: false, addEventListener: vi.fn((_name: string, listener: () => void) => { change = listener; }), removeEventListener: vi.fn() };
    vi.stubGlobal('matchMedia', vi.fn().mockReturnValue(media));
    const view = render(<ScrollCapturePreview preview={preview(400)} width={200} height={200} />);
    view.rerender(<ScrollCapturePreview preview={preview(600, 200)} width={200} height={200} />);
    expect(frames.size).toBe(1); media.matches = true; act(() => change());
    expect(frames.size).toBe(0); expect(parts(view).box.style.height).toBe('150px');
    view.rerender(<ScrollCapturePreview preview={preview(2000, 400)} width={200} height={200} />);
    expect(parts(view).marker.style.top).toBe('40px'); expect(frames.size).toBe(0);
    view.unmount(); expect(media.removeEventListener).toHaveBeenCalledWith('change', change);
  });
});
