export type Arm = 'A' | 'B' | 'C';

const chosen = (import.meta as { env?: Record<string, string | undefined> }).env?.VITE_BP_ARM;

export const ARM: Arm = chosen === 'B' || chosen === 'C' ? chosen : 'A';
export const HOLD_DOM = ARM !== 'A';
export const WORKER_PRESENTS = ARM === 'C';

export function markPictures(count: number): void {
  if (count > 0) performance.mark(`bp-pic-${count}`);
}

export function throttledFrames(
  subscribeFrame: (listener: () => void) => () => void,
  playing: () => boolean,
  everyMs = 250,
): (changed: () => void) => () => void {
  if (!HOLD_DOM) return subscribeFrame;
  return changed => {
    let last = -Infinity;
    return subscribeFrame(() => {
      const now = performance.now();
      if (playing() && now - last < everyMs) return;
      last = now;
      changed();
    });
  };
}
