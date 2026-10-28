/** Shapes ease; numbers stay exact. Easing is presentation only. */

export type MotionFeel = 'snappy' | 'smooth' | 'cinematic';
const BASE_MS: Record<MotionFeel, number> = { snappy: 90, smooth: 180, cinematic: 320 };

/** Moves `current` toward `target`, framerate-independently; tau 0 snaps, and within `within` of it, it arrives. */
export function approach(current: number, target: number, dtMs: number, tauMs: number, within = 0): number {
  if (!(tauMs > 0) || !Number.isFinite(current)) return target;
  const next = current + (target - current) * (1 - Math.exp(-Math.max(0, dtMs) / tauMs));
  return Math.abs(next - target) < Math.max(within, 1e-6 * Math.max(1, Math.abs(target))) ? target : next;
}

/** Missing values and recording gaps start afresh; only known shapes glide. */
export function approachAcrossGap(current: number | null, target: number | null, dtMs: number, tauMs: number, gap: boolean): number | null {
  return gap || current == null || target == null ? target : approach(current, target, dtMs, tauMs);
}

/** The easing time for the app's motion setting: shorter at high playback speeds so
 * shapes keep up, and none when the user asked for reduced motion. */
export function easeTau(feel: MotionFeel, hoursPerSecond: number, reduced: boolean): number {
  if (reduced) return 0;
  const speedup = 1 + Math.log10(Math.max(1, hoursPerSecond / 24));
  return BASE_MS[feel] / speedup;
}
