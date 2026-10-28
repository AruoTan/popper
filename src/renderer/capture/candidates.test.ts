import { describe, expect, it } from 'vitest';
import type { CaptureCandidate, CaptureTarget } from '../../shared/capture';
import { candidateChain } from './candidates';

const windowTarget: CaptureTarget = { rect: { x: 0, y: 0, width: 1000, height: 800 }, label: 'window', controls: [] };
const control: CaptureCandidate = { rect: { x: 10, y: 10, width: 100, height: 80 }, label: 'control' };

describe('capture candidate hierarchy', () => {
  it('keeps distinct enclosing parents for Tab and excludes overlapping siblings and duplicate bounds', () => {
    const duplicate = { ...control, label: 'duplicate' };
    const sibling = { rect: { x: 20, y: 20, width: 200, height: 200 }, label: 'overlap' };
    expect(candidateChain([windowTarget, sibling, control, duplicate], { x: 25, y: 25 }).map((item) => item.label)).toEqual(['control', 'window']);
  });
  it('uses exclusive right and bottom pixel edges for neighboring targets', () => {
    expect(candidateChain([control], { x: 110, y: 20 })).toEqual([]);
    expect(candidateChain([control], { x: 20, y: 90 })).toEqual([]);
  });
});
