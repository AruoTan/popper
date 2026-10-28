import type { CaptureCandidate, CapturePoint, CaptureRect } from '../../shared/capture';

// Hit testing uses exclusive right/bottom pixel edges. Selection handles in
// geometry.ts intentionally include those edges so boundary dragging still works.
export const sameRect = (a: CaptureRect, b: CaptureRect): boolean => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
export const containsPixel = (rect: CaptureRect, point: CapturePoint): boolean => point.x >= rect.x && point.y >= rect.y && point.x < rect.x + rect.width && point.y < rect.y + rect.height;

export function encloses(parent: CaptureRect, child: CaptureRect): boolean {
  return parent.x <= child.x && parent.y <= child.y && parent.x + parent.width >= child.x + child.width && parent.y + parent.height >= child.y + child.height;
}
export function candidateChain(items: CaptureCandidate[], point: CapturePoint): CaptureCandidate[] {
  const sorted = items.filter(({ rect }) => containsPixel(rect, point) && rect.width > 0 && rect.height > 0)
    .sort((a, b) => a.rect.width * a.rect.height - b.rect.width * b.rect.height);
  const result: CaptureCandidate[] = [];
  for (const item of sorted) {
    const last = result.at(-1);
    if (!last || (encloses(item.rect, last.rect) && !sameRect(item.rect, last.rect))) result.push(item);
  }
  return result;
}

