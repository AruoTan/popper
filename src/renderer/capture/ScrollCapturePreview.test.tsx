import { render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ScrollCapturePreview } from './ScrollCapturePreview';
import type { PreparedScrollPreview } from './scrollPreview';

const preview = (height: number, y = 0): PreparedScrollPreview => ({ url: `blob:${height}`, progress: {
  width: 800, height, revision: height, status: 'appended', viewport: { x: 0, y, width: 800, height: 200 },
} });
const animations: Array<{ currentTime: number; cancel: ReturnType<typeof vi.fn> }> = [];
const animate = vi.fn((_frames: Keyframe[], _options: KeyframeAnimationOptions) => {
  const animation = { currentTime: 0, cancel: vi.fn() }; animations.push(animation); return animation as unknown as Animation;
});
beforeEach(() => {
  animations.length = 0; animate.mockClear();
  Object.defineProperty(HTMLElement.prototype, 'animate', { configurable: true, value: animate });
});
afterEach(() => { Reflect.deleteProperty(HTMLElement.prototype, 'animate'); vi.unstubAllGlobals(); });

describe('scroll preview motion', () => {
  it('retargets from the current uniform scale and keeps viewport motion in panel pixels', () => {
    const view = render(<ScrollCapturePreview preview={preview(1000)} width={200} height={200} />);
    expect(animate).not.toHaveBeenCalled();
    view.rerender(<ScrollCapturePreview preview={preview(2000, 400)} width={200} height={200} />);
    expect(animate.mock.calls[0]).toEqual([[{ transform: 'scale(2)' }, { transform: 'scale(1)' }], { duration: 80, easing: 'linear' }]);
    const marker = view.getByLabelText('当前页面位置'); expect(marker.style.top).toBe('40px'); expect(marker.style.height).toBe('20px');
    animations[0]!.currentTime = 40;
    view.rerender(<ScrollCapturePreview preview={preview(4000, 1000)} width={200} height={200} />);
    expect(animations[0]!.cancel).toHaveBeenCalledOnce();
    const keyframes = animate.mock.calls[1]![0] as Keyframe[];
    expect(Number(keyframes[0]!.transform!.toString().slice(6, -1))).toBeCloseTo(3);
    expect(marker.style.top).toBe('50px'); expect(marker.style.height).toBe('10px');
    view.rerender(<ScrollCapturePreview preview={preview(4000, 2000)} width={200} height={200} />);
    expect(animate).toHaveBeenCalledTimes(2); expect(marker.style.top).toBe('100px');
    view.unmount(); expect(animations[1]!.cancel).toHaveBeenCalledOnce();
  });
  it('updates immediately when reduced motion is enabled', () => {
    vi.stubGlobal('matchMedia', vi.fn().mockReturnValue({ matches: true }));
    const view = render(<ScrollCapturePreview preview={preview(1000)} width={200} height={200} />);
    view.rerender(<ScrollCapturePreview preview={preview(2000, 400)} width={200} height={200} />);
    expect(animate).not.toHaveBeenCalled();
    expect(view.getByLabelText('当前页面位置').style.top).toBe('40px'); view.unmount();
  });
});
