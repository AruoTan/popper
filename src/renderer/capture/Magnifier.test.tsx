import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Magnifier } from './Magnifier';

describe('capture magnifier', () => {
  it.each([[100, 100], [0, 0], [1999, 0], [0, 1399], [2000, 1400]])('keeps pixel sampling centered at (%s, %s) with 3x zoom and a solid crosshair', (x, y) => {
    const context = {
      fillRect: vi.fn(), drawImage: vi.fn(), beginPath: vi.fn(), moveTo: vi.fn(), lineTo: vi.fn(), stroke: vi.fn(), strokeRect: vi.fn(),
      getImageData: vi.fn(() => ({ data: new Uint8ClampedArray([35, 39, 46, 255]) })),
      imageSmoothingEnabled: true, lineWidth: 0, strokeStyle: '',
    };
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context as unknown as CanvasRenderingContext2D);
    const image = new Image(), screen = { x: -2000, y: -1400, width: 2000, height: 1400 };
    const view = render(<Magnifier image={image} point={{ x, y }} screen={screen} viewport={{ width: 1000, height: 700 }} />);
    const px = Math.min(1999, x), py = Math.min(1399, y), left = Math.max(0, px - 22), top = Math.max(0, py - 22);
    const width = Math.min(2000, px + 23) - left, height = Math.min(1400, py + 23) - top;
    expect(context.drawImage).toHaveBeenLastCalledWith(image, left, top, width, height,
      65 + (left - px - .5) * 3, 65 + (top - py - .5) * 3, width * 3, height * 3);
    expect(context.imageSmoothingEnabled).toBe(false);
    expect(context.getImageData).toHaveBeenCalledWith(65, 65, 1, 1);
    expect(context.getImageData.mock.invocationCallOrder[0]).toBeLessThan(context.stroke.mock.invocationCallOrder[0]!);
    expect(context.moveTo.mock.calls).toEqual([[0, 65.5], [65.5, 0]]);
    expect(context.lineTo.mock.calls).toEqual([[130, 65.5], [65.5, 130]]);
    expect(context.strokeRect).not.toHaveBeenCalled();
    expect(context.strokeStyle).toBe('#14c36e');
    expect(view.getByLabelText('像素放大预览')).toHaveTextContent(`${screen.x + px}, ${screen.y + py}`);
    expect(view.getByText('#23272E')).toBeInTheDocument();
    expect(view.getByText('按 Ctrl + C 复制色值')).toBeInTheDocument();
    expect(view.getByText('按 Shift 切换 RGB/HEX')).toBeInTheDocument();
    view.rerender(<Magnifier image={image} point={{ x, y }} screen={screen} viewport={{ width: 1000, height: 700 }} format="rgb" />);
    expect(view.getByText('35, 39, 46')).toBeInTheDocument();
    expect(context.drawImage).toHaveBeenCalledTimes(1);
    view.unmount();
  });
});
