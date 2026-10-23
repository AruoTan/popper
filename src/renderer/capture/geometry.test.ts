import { describe, expect, it } from 'vitest';
import type { CaptureAnnotation } from '../../shared/capture';
import { commit, constrainPoint, emptyHistory, intersect, keyRect, moveRect, rectangle, redo, resizeRect, undo } from './geometry';

const bounds = { x: 0, y: 0, width: 3840, height: 2160 };
describe('screen pixel selection', () => {
  it('normalizes reverse drags and clips to a single screen', () => {
    const start = constrainPoint({ x: 400, y: 500 }, bounds);
    const end = constrainPoint({ x: -100, y: 2500 }, bounds);
    expect(rectangle(start, end)).toEqual({ x: 0, y: 500, width: 400, height: 1660 });
    expect(intersect({ x: -1920, y: -200, width: 1920, height: 1080 }, { x: -2000, y: -300, width: 400, height: 400 })).toEqual({ x: -1920, y: -200, width: 320, height: 300 });
  });
  it('keeps one-pixel keyboard moves independent of monitor scaling', () => {
    const rect = { x: 100, y: 200, width: 1000, height: 500 };
    expect(keyRect(rect, 'ArrowRight', false, false, bounds)).toEqual({ ...rect, x: 101 });
    expect(keyRect(rect, 'ArrowLeft', true, false, bounds)).toEqual({ ...rect, x: 99, width: 1001 });
    expect(keyRect(rect, 'ArrowDown', false, true, bounds)).toEqual({ ...rect, height: 499 });
    expect(keyRect({ x: 0, y: 0, width: 1, height: 1 }, 'ArrowUp', false, true, bounds)).toEqual({ x: 0, y: 0, width: 1, height: 1 });
  });
  it('constrains movement and all resize handles without rescaling image content', () => {
    const rect = { x: 100, y: 100, width: 200, height: 200 };
    expect(moveRect(rect, 9000, -9000, bounds)).toEqual({ x: 3640, y: 0, width: 200, height: 200 });
    expect(resizeRect(rect, 'nw', { x: 500, y: 500 }, bounds)).toEqual({ x: 299, y: 299, width: 1, height: 1 });
    expect(resizeRect(rect, 'se', { x: 9000, y: 9000 }, bounds)).toEqual({ x: 100, y: 100, width: 3740, height: 2060 });
  });
});
describe('annotation history', () => {
  it('restores object content and invalidates redo after a new edit', () => {
    const original: CaptureAnnotation = { id: 'text-1', tool: 'text', color: '#ff0000', width: 3, points: [{ x: 100, y: 100 }], text: '原始文字' };
    const first = commit(emptyHistory(), [original]);
    const second = commit(first, [{ ...original, text: '修改文字', points: [{ x: 200, y: 100 }] }]);
    const reverted = undo(second);
    expect(reverted.present).toEqual([original]);
    expect(redo(reverted).present[0]?.text).toBe('修改文字');
    const replacement = commit(reverted, []);
    expect(replacement.future).toEqual([]);
    expect(redo(replacement).present).toEqual([]);
    expect(first.present[0]?.points[0]?.x).toBe(100);
  });
});
