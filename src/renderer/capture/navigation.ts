import type { CaptureCandidate, CapturePoint, CaptureRect } from '../../shared/capture';
import { candidateChain } from './hover';

const sameRect = (a: CaptureRect, b: CaptureRect): boolean => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
const encloses = (a: CaptureRect, b: CaptureRect): boolean => a.x <= b.x && a.y <= b.y && a.x + a.width >= b.x + b.width && a.y + a.height >= b.y + b.height;
const contains = (rect: CaptureRect, point: CapturePoint): boolean => point.x >= rect.x && point.y >= rect.y && point.x < rect.x + rect.width && point.y < rect.y + rect.height;

/** Explicit hierarchy choices survive asynchronous refinement and small pointer jitter. */
export function createCaptureNavigation(): {
  update: (items: CaptureCandidate[], point: CapturePoint, target: number, scale: CapturePoint) => { items: CaptureCandidate[]; index: number };
  cycle: (direction: number) => { items: CaptureCandidate[]; index: number };
} {
  let items: CaptureCandidate[] = [], index = 0, point = { x: 0, y: 0 }, target = -1;
  let manual: { candidate: CaptureCandidate; point: CapturePoint; target: number } | null = null;
  return {
    update(next, at, nextTarget, scale) {
      if (manual && (manual.target !== nextTarget || !contains(manual.candidate.rect, at) || Math.hypot((at.x - manual.point.x) * scale.x, (at.y - manual.point.y) * scale.y) > 4)) manual = null;
      const chosen = manual?.candidate;
      items = candidateChain(chosen ? [...next.filter((item) => encloses(item.rect, chosen.rect) || encloses(chosen.rect, item.rect)), chosen] : next, at);
      index = chosen ? Math.max(0, items.findIndex((item) => sameRect(item.rect, chosen.rect))) : 0;
      point = at; target = nextTarget;
      return { items, index };
    },
    cycle(direction) {
      index = Math.max(0, Math.min(items.length - 1, index + direction));
      const candidate = items[index];
      if (candidate) manual = { candidate, point, target };
      return { items, index };
    },
  };
}
