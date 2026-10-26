import { createRef, type ReactNode } from 'react';
import { act, fireEvent, render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PinEditor, type PinEditorHandle } from './PinEditor';
import { pngBytes, renderScene } from './drawing';
import type { CaptureAnnotation } from '../../shared/capture';

vi.mock('./drawing', () => ({ renderScene: vi.fn(), pngBytes: vi.fn(async () => new Uint8Array([1, 2, 3])) }));
const image = { naturalWidth: 800, naturalHeight: 480 } as HTMLImageElement;
const initial: CaptureAnnotation[] = [{ id: 'original', tool: 'rectangle', points: [{ x: 40.5, y: 20.5 }, { x: 120.5, y: 100.5 }], color: '#ff4545', width: 3 }];
beforeEach(() => vi.clearAllMocks());

function setup(items: CaptureAnnotation[] = []) {
  const ref = createRef<PinEditorHandle>();
  const props = { ref, image, original: new Uint8Array([8, 9]).buffer, initial: items, toolbar: true, visible: true, busy: false, onOutput: vi.fn(), onHideToolbar: vi.fn(), onError: vi.fn() };
  const pointerDown = vi.fn(), wheel = vi.fn();
  const shell = (child: ReactNode) => <div onPointerDown={pointerDown} onWheel={wheel}>{child}</div>;
  const view = render(shell(<PinEditor {...props} />));
  const canvas = view.getByLabelText('贴图标注画布') as HTMLCanvasElement;
  Object.assign(canvas, { setPointerCapture: vi.fn(), hasPointerCapture: () => true, releasePointerCapture: vi.fn() });
  vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue({ left: 24, top: 24, width: 400, height: 240 } as DOMRect);
  return { ...view, ref, props, canvas, pointerDown, wheel, rerender: (child: ReactNode) => view.rerender(shell(child)) };
}
function draw(canvas: HTMLCanvasElement, shiftKey = false) {
  fireEvent.pointerDown(canvas, { button: 0, pointerId: 1, clientX: 44, clientY: 44 });
  fireEvent.pointerMove(canvas, { pointerId: 1, clientX: 124, clientY: 84, shiftKey });
  fireEvent.pointerUp(canvas, { pointerId: 1, clientX: 124, clientY: 84 });
}
const renderedItems = () => vi.mocked(renderScene).mock.calls.at(-1)?.[4] ?? [];

describe('pin annotation layers', () => {
  it('hides initial and edited annotations without discarding them, and exports the visible layer at original dimensions', async () => {
    const view = setup(initial);
    expect(renderedItems()).toEqual(initial);
    let bytes!: Uint8Array;
    await act(async () => { bytes = await view.ref.current!.snapshot(); });
    expect(bytes).toEqual(new Uint8Array([1, 2, 3]));
    expect(renderScene).toHaveBeenLastCalledWith(expect.any(HTMLCanvasElement), image, 800, 480, initial);
    view.rerender(<PinEditor {...view.props} visible={false} />);
    await act(async () => { bytes = await view.ref.current!.snapshot(); });
    expect(bytes).toEqual(new Uint8Array([8, 9]));
    view.rerender(<PinEditor {...view.props} />);
    expect(renderedItems()).toEqual(initial);
    view.unmount();
  });

  it.each([['矩形', 'rectangle'], ['椭圆', 'ellipse'], ['箭头', 'arrow'], ['画笔', 'pen'], ['马赛克', 'mosaic']])('draws %s in original pixels and supports undo, redo and delete', (label, tool) => {
    const view = setup();
    fireEvent.click(view.getByRole('button', { name: label })); draw(view.canvas);
    expect(renderedItems()).toEqual([expect.objectContaining({ tool, points: expect.arrayContaining([{ x: 40, y: 40 }, { x: 200, y: 120 }]) })]);
    expect(view.getByRole('button', { name: '撤销' })).toBeEnabled();
    fireEvent.click(view.getByRole('button', { name: '撤销' }));
    expect(view.getByRole('button', { name: '重做' })).toBeEnabled();
    fireEvent.click(view.getByRole('button', { name: '重做' }));
    expect(renderedItems()).toHaveLength(1);
    fireEvent.click(view.getByRole('button', { name: '选择与调整' }));
    fireEvent.pointerDown(view.canvas, { button: 0, pointerId: 2, clientX: 70, clientY: 60 });
    fireEvent.pointerUp(view.canvas, { pointerId: 2 });
    fireEvent.click(view.getByRole('button', { name: '删除标注' }));
    expect(view.getByRole('button', { name: '删除标注' })).toBeDisabled();
    view.unmount();
  });

  it('constrains shapes, adjusts pen width with the wheel and prevents image zoom', () => {
    const view = setup();
    fireEvent.click(view.getByRole('button', { name: '矩形' }));
    fireEvent.wheel(view.canvas, { deltaY: -120 });
    expect(view.getByLabelText('线宽')).toHaveValue(4);
    expect(view.wheel).not.toHaveBeenCalled();
    draw(view.canvas, true);
    const points = renderedItems()[0]!.points;
    expect(Math.abs(points[1]!.x - points[0]!.x)).toBe(Math.abs(points[1]!.y - points[0]!.y));
    view.unmount();
  });

  it('commits Chinese text on output, and allows editing it again', async () => {
    const view = setup();
    fireEvent.click(view.getByRole('button', { name: '文字' }));
    fireEvent.pointerDown(view.canvas, { button: 0, clientX: 44, clientY: 44 });
    fireEvent.change(view.getByLabelText('标注文字'), { target: { value: '中文标注\n第二行' } });
    await act(async () => { await view.ref.current!.snapshot(); });
    expect(renderedItems()).toEqual([expect.objectContaining({ tool: 'text', text: '中文标注\n第二行' })]);
    fireEvent.doubleClick(view.canvas, { clientX: 44, clientY: 44 });
    expect(view.getByLabelText('标注文字')).toHaveValue('中文标注\n第二行');
    view.unmount();
  });

  it('cancels pending gestures when the toolbar is hidden and keeps the image draggable', () => {
    const view = setup();
    fireEvent.click(view.getByRole('button', { name: '矩形' }));
    fireEvent.pointerDown(view.canvas, { button: 0, pointerId: 1, clientX: 44, clientY: 44 });
    fireEvent.pointerMove(view.canvas, { pointerId: 1, clientX: 124, clientY: 84 });
    view.rerender(<PinEditor {...view.props} toolbar={false} />);
    fireEvent.pointerUp(view.canvas, { pointerId: 1 });
    expect(view.queryByRole('toolbar')).toBeNull();
    fireEvent.pointerDown(view.canvas, { button: 0, pointerId: 2 });
    expect(view.pointerDown).toHaveBeenCalledTimes(1);
    view.rerender(<PinEditor {...view.props} />);
    expect(view.getByRole('button', { name: '撤销' })).toBeDisabled();
    view.unmount();
  });

  it('does not create empty shapes or add undo steps when an object is only selected', () => {
    const view = setup(initial);
    fireEvent.click(view.getByRole('button', { name: '矩形' }));
    fireEvent.pointerDown(view.canvas, { button: 0, pointerId: 1, clientX: 200, clientY: 100 });
    fireEvent.pointerUp(view.canvas, { pointerId: 1 });
    expect(view.getByRole('button', { name: '撤销' })).toBeDisabled();
    fireEvent.click(view.getByRole('button', { name: '选择与调整' }));
    fireEvent.pointerDown(view.canvas, { button: 0, pointerId: 2, clientX: 50, clientY: 45 });
    fireEvent.pointerUp(view.canvas, { pointerId: 2 });
    expect(view.getByRole('button', { name: '删除标注' })).toBeEnabled();
    expect(view.getByRole('button', { name: '撤销' })).toBeDisabled();
    view.unmount();
  });
});
