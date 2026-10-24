import type { CaptureCandidate, CaptureHitTest, CapturePoint, CaptureRect, CaptureTarget } from '../../shared/capture';
const contains = (rect: CaptureRect, point: CapturePoint): boolean => point.x >= rect.x && point.y >= rect.y && point.x < rect.x + rect.width && point.y < rect.y + rect.height;

function encloses(parent: CaptureRect, child: CaptureRect): boolean {
  return parent.x <= child.x && parent.y <= child.y && parent.x + parent.width >= child.x + child.width && parent.y + parent.height >= child.y + child.height;
}
export function candidateChain(items: CaptureCandidate[], point: CapturePoint): CaptureCandidate[] {
  const sorted = items.filter(({ rect }) => contains(rect, point) && rect.width > 0 && rect.height > 0)
    .sort((a, b) => a.rect.width * a.rect.height - b.rect.width * b.rect.height);
  const result: CaptureCandidate[] = [];
  for (const item of sorted) {
    const last = result.at(-1);
    if (!last || (encloses(item.rect, last.rect) && (item.rect.x !== last.rect.x || item.rect.y !== last.rect.y || item.rect.width !== last.rect.width || item.rect.height !== last.rect.height))) result.push(item);
  }
  return result;
}

/** Immediate geometry feedback with one active provider request and one latest point. */
export function createCaptureHover(options: {
  targets: CaptureTarget[];
  active: () => boolean;
  request: (point: CapturePoint) => Promise<CaptureHitTest>;
  show: (candidates: CaptureCandidate[], context: { point: CapturePoint; target: number }) => void;
}): { update: (point: CapturePoint) => void; destroy: () => void } {
  let point: CapturePoint | null = null, timer: ReturnType<typeof setTimeout> | undefined;
  let inFlight = false, disposed = false, revision = 0, retries = 0;
  const cache: { target: number; candidates: CaptureCandidate[] }[] = [];
  const targetAt = (at: CapturePoint): number => options.targets.findIndex((target) => contains(target.rect, at));
  const show = (): void => {
    if (!point || disposed || !options.active()) return;
    const index = targetAt(point), target = options.targets[index];
    // Check the captured z order first; cached controls from covered windows cannot win.
    const items = cache.filter((entry) => entry.target === index).flatMap((entry) => entry.candidates);
    if (target) items.push(...target.controls, { rect: target.rect, label: target.label });
    options.show(candidateChain(items, point), { point, target: index });
  };
  const schedule = (delay = 16): void => {
    if (disposed || inFlight || timer !== undefined || !options.active()) return;
    timer = setTimeout(() => { timer = undefined; void run(); }, delay);
  };
  const run = async (): Promise<void> => {
    if (!point || disposed || !options.active()) return;
    const at = point, requested = revision, target = targetAt(at);
    inFlight = true;
    let retry = false;
    try {
      const { candidates, pending } = await options.request(at);
      if (disposed || !options.active()) return;
      cache.push({ target, candidates });
      if (cache.length > 32) cache.shift();
      // A moved pointer can still use valid bounds from this response immediately.
      // New positions are refined next without another debounce period.
      show();
      retry = pending;
    } catch { /* Native geometry remains available when a provider fails. */ }
    finally {
      inFlight = false;
      if (!disposed && requested !== revision) schedule();
      else if (retry && retries++ < 2) schedule(32);
    }
  };
  return {
    update(at) {
      if (disposed || !options.active() || (point?.x === at.x && point.y === at.y)) return;
      point = at; revision++; retries = 0; show(); schedule();
    },
    destroy() { disposed = true; clearTimeout(timer); cache.length = 0; },
  };
}
