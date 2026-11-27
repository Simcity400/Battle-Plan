import type { Value } from './value.ts';
import type { ValueFormat } from './valueFormat.ts';

export interface MotionClock {
  readonly playing: boolean;
  readonly anchorPosition: number;
  readonly anchorTimeMs: number;
  readonly speed: number;
  readonly seek: number;
  readonly exactHour: number;
  readonly branch: number;
  readonly settleStartMs: number;
  readonly settleDurationMs: number;
  readonly lastHour?: number;
}

export type MotionMode = 'playing' | 'settling' | 'exact';

export function positionAt(clock: MotionClock, wallNowMs: number): number {
  if (!clock.playing || !(clock.speed > 0)) return clock.anchorPosition;
  const elapsedS = Math.max(0, wallNowMs - clock.anchorTimeMs) / 1000;
  const raw = clock.anchorPosition + elapsedS * clock.speed;
  return clock.lastHour !== undefined ? Math.min(clock.lastHour, raw) : raw;
}

export function easeOutCubicMotion(t: number): number {
  const u = 1 - Math.max(0, Math.min(1, t));
  return 1 - u * u * u;
}

function formatScaleFactor(format: ValueFormat | undefined): number {
  if (format === 'fixed5Amount' || format === 'fixed5Width' || format === 'fixed5Percent') return 100000;
  if (typeof format === 'object' && format !== null && format.decimals > 0) return 10 ** format.decimals;
  return 1;
}

export function interpolateFamilyValue(
  original: Value,
  interp: number,
  options?: { readonly originalNumeric?: number; readonly format?: ValueFormat },
): Value {
  if (original.kind !== 'known' && original.kind !== 'zero' && original.kind !== 'partial') return original;
  const raw = original.value;
  if (typeof raw === 'string') return original;
  const origNum = options?.originalNumeric;
  const fmtScale = formatScaleFactor(options?.format);
  let nextRaw: number | bigint;
  if (typeof raw === 'bigint') {
    const scale = origNum !== undefined && origNum !== 0 ? Number(raw) / origNum : fmtScale;
    const rounded = Math.round(interp * scale);
    nextRaw = BigInt(rounded === 0 ? 0 : rounded);
  } else {
    const scale = origNum !== undefined && origNum !== 0 ? raw / origNum : fmtScale;
    const scaled = Math.abs(scale - 1) > 1e-6 ? Math.round(interp * scale) : interp;
    nextRaw = Object.is(scaled, -0) ? 0 : scaled;
  }
  if (original.kind === 'partial') {
    return { ...original, value: nextRaw };
  }
  if (nextRaw === 0 || nextRaw === 0n) {
    return { kind: 'zero', value: typeof raw === 'bigint' ? 0n : 0 };
  }
  return { kind: 'known', value: nextRaw };
}

export interface MotionStep {
  readonly mode: MotionMode;
  readonly continuousHour: number;
  readonly bracketT: number | null;
  readonly settleProgress: number;
  readonly dtMs: number;
  readonly cut: boolean;
  readonly justSettled: boolean;
  readonly settleElapsedMs: number;
}

export interface MotionFieldStats {
  bracketFrames: number;
  chaseFrames: number;
  gapSnaps: number;
  settleCount: number;
  lastSettleMs: number;
}

const SCALAR_CHASE_TAU_MS = 85;
const BRACKET_BLEND_TAU_MS = 45;
const Y_GLIDE_TAU_MS = 110;
const EPSILON = 1e-4;

export class MotionField {
  private readonly scalars = new Map<string, number>();
  private readonly ys = new Map<string, number>();
  private readonly settleStartScalars = new Map<string, number>();
  private readonly settleStartYs = new Map<string, number>();
  private lastWallMs = -Infinity;
  private lastSeek = -1;
  private lastBranch = -1;
  private wasPlaying = false;
  private settling = false;
  private settleStartMs = 0;
  private settleDurationMs = 0;
  private frameDirty = true;
  private pendingMotion = false;
  private hadPendingMotion = false;
  private currentStep: MotionStep = {
    mode: 'exact',
    continuousHour: 0,
    bracketT: null,
    settleProgress: 1,
    dtMs: 0,
    cut: true,
    justSettled: false,
    settleElapsedMs: 0,
  };
  readonly stats: MotionFieldStats = {
    bracketFrames: 0,
    chaseFrames: 0,
    gapSnaps: 0,
    settleCount: 0,
    lastSettleMs: 0,
  };

  get isSettling(): boolean {
    return this.settling;
  }

  get dirty(): boolean {
    return this.frameDirty;
  }

  get active(): boolean {
    return this.hadPendingMotion || this.pendingMotion || this.settling;
  }

  get step(): MotionStep {
    return this.currentStep;
  }

  reset(): void {
    this.scalars.clear();
    this.ys.clear();
    this.settleStartScalars.clear();
    this.settleStartYs.clear();
    this.settling = false;
    this.pendingMotion = false;
    this.hadPendingMotion = false;
  }

  beginFrame(
    clock: MotionClock,
    wallNowMs: number,
    bracketStart: number,
    bracketEnd: number | null,
    hasGap: boolean,
    offset = 0,
    cappedPosition?: number,
  ): MotionStep {
    const dtMs = Number.isFinite(this.lastWallMs) ? Math.max(1, Math.min(100, wallNowMs - this.lastWallMs)) : 16.67;
    this.lastWallMs = wallNowMs;

    const branchChanged = this.lastBranch !== -1 && clock.branch !== this.lastBranch;
    const seekChanged = this.lastSeek !== -1 && clock.seek !== this.lastSeek;
    const cut = branchChanged || seekChanged || hasGap;

    this.lastBranch = clock.branch;
    this.lastSeek = clock.seek;

    if (cut) {
      if (branchChanged || hasGap) this.stats.gapSnaps++;
      this.reset();
    }

    const continuousHour = cappedPosition !== undefined ? cappedPosition : positionAt(clock, wallNowMs) + offset;
    let mode: MotionMode = 'exact';
    let bracketT: number | null = null;
    let settleProgress = 1;
    let justSettled = false;
    let settleElapsedMs = 0;

    if (clock.playing) {
      this.wasPlaying = true;
      this.settling = false;
      mode = 'playing';
      if (!cut && bracketEnd !== null && bracketEnd > bracketStart && continuousHour >= bracketStart && continuousHour <= bracketEnd) {
        bracketT = Math.max(0, Math.min(1, (continuousHour - bracketStart) / (bracketEnd - bracketStart)));
        this.stats.bracketFrames++;
      } else {
        this.stats.chaseFrames++;
      }
    } else {
      if (this.wasPlaying && !cut && clock.settleDurationMs > 0 && (this.scalars.size > 0 || this.ys.size > 0)) {
        this.wasPlaying = false;
        this.settling = true;
        this.settleStartMs = clock.settleStartMs > 0 ? clock.settleStartMs : wallNowMs;
        this.settleDurationMs = clock.settleDurationMs;
        this.settleStartScalars.clear();
        for (const [k, v] of this.scalars) this.settleStartScalars.set(k, v);
        this.settleStartYs.clear();
        for (const [k, v] of this.ys) this.settleStartYs.set(k, v);
      } else if (this.wasPlaying) {
        this.wasPlaying = false;
        if (!this.settling) justSettled = true;
      }

      if (this.settling) {
        settleElapsedMs = Math.max(0, wallNowMs - this.settleStartMs);
        if (settleElapsedMs >= this.settleDurationMs) {
          this.settling = false;
          this.settleStartScalars.clear();
          this.settleStartYs.clear();
          mode = 'exact';
          settleProgress = 1;
          justSettled = true;
          this.stats.settleCount++;
          this.stats.lastSettleMs = settleElapsedMs;
        } else {
          mode = 'settling';
          settleProgress = settleElapsedMs / this.settleDurationMs;
        }
      } else {
        mode = 'exact';
      }
    }

    this.hadPendingMotion = !cut && mode !== 'exact' && this.pendingMotion;
    this.frameDirty = cut || justSettled || mode === 'settling';
    this.pendingMotion = mode === 'settling';
    this.currentStep = {
      mode,
      continuousHour: mode === 'exact' ? clock.exactHour + offset : continuousHour,
      bracketT,
      settleProgress,
      dtMs,
      cut,
      justSettled,
      settleElapsedMs,
    };
    return this.currentStep;
  }

  stepScalar(
    key: string,
    h0Val: number | null,
    h1Val: number | null | undefined,
    gapOrMissing = false,
    easeFn: (t: number) => number = easeOutCubicMotion,
  ): number | null {
    const step = this.currentStep;
    const h0Missing = h0Val === null || !Number.isFinite(h0Val);
    const h1Missing = h1Val !== undefined && (h1Val === null || !Number.isFinite(h1Val));

    if (gapOrMissing || step.cut || h0Missing || h1Missing) {
      if (gapOrMissing || (h0Missing && this.scalars.has(key)) || h1Missing) {
        this.stats.gapSnaps++;
      }
      if (this.scalars.has(key)) this.frameDirty = true;
      this.scalars.delete(key);
      this.settleStartScalars.delete(key);
      return h0Missing ? null : h0Val;
    }

    if (step.mode === 'exact') {
      const prev = this.scalars.get(key);
      if (prev === undefined || Math.abs(h0Val - prev) > EPSILON) this.frameDirty = true;
      this.scalars.set(key, h0Val);
      return h0Val;
    }

    if (step.mode === 'settling') {
      const start = this.settleStartScalars.get(key);
      if (start === undefined || !Number.isFinite(start)) {
        this.scalars.set(key, h0Val);
        return h0Val;
      }
      const next = start + (h0Val - start) * easeFn(step.settleProgress);
      const prev = this.scalars.get(key);
      if (prev === undefined || Math.abs(next - prev) > EPSILON) this.frameDirty = true;
      this.scalars.set(key, next);
      return next;
    }

    const prev = this.scalars.get(key);
    if (h1Val !== undefined && step.bracketT !== null) {
      const target = h0Val + (h1Val - h0Val) * step.bracketT;
      if (Math.abs(h1Val - h0Val) > EPSILON) this.pendingMotion = true;
      if (prev === undefined || !Number.isFinite(prev) || Math.abs(target - prev) <= EPSILON) {
        if (prev === undefined || !Number.isFinite(prev) || Math.abs(target - prev) > EPSILON) this.frameDirty = true;
        this.scalars.set(key, target);
        return target;
      }
      const alpha = 1 - Math.exp(-step.dtMs / BRACKET_BLEND_TAU_MS);
      const lo = Math.min(h0Val, h1Val);
      const hi = Math.max(h0Val, h1Val);
      const next = Math.max(lo, Math.min(hi, prev + (target - prev) * alpha));
      if (Math.abs(next - prev) > EPSILON) this.frameDirty = true;
      if (Math.abs(target - next) > EPSILON) this.pendingMotion = true;
      this.scalars.set(key, next);
      return next;
    }

    if (prev === undefined || !Number.isFinite(prev)) {
      this.frameDirty = true;
      this.scalars.set(key, h0Val);
      return h0Val;
    }
    const alpha = 1 - Math.exp(-step.dtMs / SCALAR_CHASE_TAU_MS);
    const next = Math.abs(h0Val - prev) <= EPSILON ? h0Val : prev + (h0Val - prev) * alpha;
    if (Math.abs(next - prev) > EPSILON) this.frameDirty = true;
    if (Math.abs(h0Val - next) > EPSILON) this.pendingMotion = true;
    this.scalars.set(key, next);
    return next;
  }

  stepY(
    key: string,
    targetY: number,
    maxJumpPx = Infinity,
    easeFn: (t: number) => number = easeOutCubicMotion,
  ): number {
    const step = this.currentStep;
    if (step.cut || step.mode === 'exact') {
      const prev = this.ys.get(key);
      if (prev === undefined || Math.abs(targetY - prev) > 0.25) this.frameDirty = true;
      this.ys.set(key, targetY);
      return targetY;
    }
    if (step.mode === 'settling') {
      const start = this.settleStartYs.get(key);
      if (start === undefined || Math.abs(targetY - start) <= 0.5) {
        this.ys.set(key, targetY);
        return targetY;
      }
      const next = start + (targetY - start) * easeFn(step.settleProgress);
      this.frameDirty = true;
      this.ys.set(key, next);
      return next;
    }
    const prev = this.ys.get(key);
    if (prev === undefined || Math.abs(targetY - prev) > maxJumpPx) {
      this.frameDirty = true;
      this.ys.set(key, targetY);
      return targetY;
    }
    if (Math.abs(targetY - prev) <= 0.25) {
      this.ys.set(key, targetY);
      return targetY;
    }
    const alpha = 1 - Math.exp(-step.dtMs / Y_GLIDE_TAU_MS);
    const next = prev + (targetY - prev) * alpha;
    this.frameDirty = true;
    if (Math.abs(targetY - next) > 0.25) this.pendingMotion = true;
    this.ys.set(key, next);
    return next;
  }

  prune(activeScalarKeys: ReadonlySet<string>, activeYKeys?: ReadonlySet<string>): void {
    for (const k of this.scalars.keys()) {
      if (!activeScalarKeys.has(k)) this.scalars.delete(k);
    }
    if (activeYKeys) {
      for (const k of this.ys.keys()) {
        if (!activeYKeys.has(k)) this.ys.delete(k);
      }
    }
  }
}
