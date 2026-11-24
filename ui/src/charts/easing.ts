
export type MotionFeel = 'snappy' | 'smooth' | 'cinematic';
const BASE_MS: Record<MotionFeel, number> = { snappy: 90, smooth: 180, cinematic: 320 };

export function approach(current: number, target: number, dtMs: number, tauMs: number, within = 0): number {
  if (!(tauMs > 0) || !Number.isFinite(current)) return target;
  const next = current + (target - current) * (1 - Math.exp(-Math.max(0, dtMs) / tauMs));
  return Math.abs(next - target) < Math.max(within, 1e-6 * Math.max(1, Math.abs(target))) ? target : next;
}

export const motionOf = (playback: { playing?: boolean; hoursPerSecond: number }): number | undefined =>
  playback.playing ? playback.hoursPerSecond : undefined;

export function easeTau(feel: MotionFeel, hoursPerSecond: number | undefined, reduced: boolean): number {
  if (reduced || hoursPerSecond === undefined) return 0;
  const speedup = 1 + Math.log10(Math.max(1, hoursPerSecond / 24));
  return BASE_MS[feel] / speedup;
}
