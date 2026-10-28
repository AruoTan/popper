import type { CaptureCandidate, CapturePoint } from '../../shared/capture';
import { candidateChain, containsPixel, encloses, sameRect } from './candidates';

/** Explicit hierarchy choices survive asynchronous refinement and small pointer jitter. */
export function createCaptureNavigation(): {
  update: (items: CaptureCandidate[], point: CapturePoint, target: number, scale: CapturePoint, pending?: boolean) => { items: CaptureCandidate[]; index: number };
  cycle: (direction: number) => { items: CaptureCandidate[]; index: number };
} {
  let items: CaptureCandidate[] = [], index = 0, point = { x: 0, y: 0 }, target = -1;
  let recognizing = false;
  let manual: { candidate: CaptureCandidate | undefined; point: CapturePoint; target: number; pending: number } | null = null;
  return {
    update(next, at, nextTarget, scale, pending = false) {
      if (manual && (manual.target !== nextTarget || (manual.candidate && !containsPixel(manual.candidate.rect, at)) || Math.hypot((at.x - manual.point.x) * scale.x, (at.y - manual.point.y) * scale.y) > 4)) manual = null;
      const chosen = manual?.candidate;
      items = candidateChain(chosen ? [...next.filter((item) => encloses(item.rect, chosen.rect) || encloses(chosen.rect, item.rect)), chosen] : next, at);
      index = chosen ? Math.max(0, items.findIndex((item) => sameRect(item.rect, chosen.rect))) : 0;
      if (manual && items.length) {
        const requested = index + manual.pending;
        index = Math.max(0, Math.min(items.length - 1, requested));
        manual.pending = pending ? requested - index : 0;
        manual.candidate = items[index];
      }
      if (manual && !pending) manual.pending = 0;
      point = at; target = nextTarget; recognizing = pending;
      return { items, index };
    },
    cycle(direction) {
      const requested = index + (manual?.pending ?? 0) + direction;
      index = Math.max(0, Math.min(items.length - 1, requested));
      const candidate = items[index];
      manual = { candidate, point, target, pending: recognizing ? Math.max(-64, Math.min(64, requested - index)) : 0 };
      return { items, index };
    },
  };
}
