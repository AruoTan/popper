import { describe, expect, it } from 'vitest';
import type { CaptureCandidate } from '../../shared/capture';
import { createCaptureNavigation } from './navigation';

const leaf: CaptureCandidate = { rect: { x: 10, y: 10, width: 100, height: 100 }, label: 'control' };
const root: CaptureCandidate = { rect: { x: 0, y: 0, width: 1000, height: 800 }, label: 'window' };
const text: CaptureCandidate = { rect: { x: 15, y: 15, width: 40, height: 40 }, label: 'text' };
const point = { x: 20, y: 20 }, scale = { x: 1, y: 1 };

describe('capture hierarchy navigation', () => {
  it('retains the chosen parent by bounds when asynchronous refinement inserts new children', () => {
    const navigation = createCaptureNavigation();
    navigation.update([leaf, root], point, 0, scale);
    expect(navigation.cycle(1).index).toBe(1);
    const refined = navigation.update([text, leaf, root], point, 0, scale);
    expect(refined.items[refined.index]).toEqual(root);
    expect(refined.index).toBe(2);
    const child = navigation.cycle(-1);
    expect(child.items[child.index]).toEqual(leaf);
  });
  it('does not discard a chosen control when a partial provider response omits it', () => {
    const navigation = createCaptureNavigation();
    navigation.update([text, leaf, root], point, 0, scale); navigation.cycle(1);
    const partial = navigation.update([root], point, 0, scale);
    expect(partial.items[partial.index]).toEqual(leaf);
  });
  it('tolerates four logical pixels of jitter with high DPI and resumes detection after a deliberate move', () => {
    const navigation = createCaptureNavigation(), dpiScale = { x: 0.5, y: 0.5 };
    navigation.update([text, leaf, root], point, 0, dpiScale); navigation.cycle(2);
    const jitter = navigation.update([text, leaf, root], { x: 26, y: 20 }, 0, dpiScale);
    expect(jitter.items[jitter.index]).toEqual(root);
    const moved = navigation.update([text, leaf, root], { x: 30, y: 20 }, 0, dpiScale);
    expect(moved.items[moved.index]).toEqual(text);
  });
  it('releases the chosen parent on crossing into a different captured window', () => {
    const navigation = createCaptureNavigation();
    navigation.update([leaf, root], point, 0, scale); navigation.cycle(1);
    const switched = navigation.update([text], point, 1, scale);
    expect(switched.items[switched.index]).toEqual(text);
  });
});
