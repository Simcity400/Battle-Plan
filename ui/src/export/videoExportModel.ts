import type { CropRect, RecordedStretch, UnheldGap } from './videoExportApi';

export interface DimensionPreset {
  id: string;
  label: string;
  width: number;
  height: number;
}

export const DIMENSION_PRESETS: readonly DimensionPreset[] = [
  { id: '720p', label: '720p (1280×720)', width: 1280, height: 720 },
  { id: '1080p', label: '1080p (1920×1080)', width: 1920, height: 1080 },
  { id: '1440p', label: '1440p (2560×1440)', width: 2560, height: 1440 },
  { id: '4k', label: '4K (3840×2160)', width: 3840, height: 2160 },
  { id: 'vertical', label: 'Vertical 9:16 (1080×1920)', width: 1080, height: 1920 },
  { id: 'square', label: 'Square 1:1 (1080×1080)', width: 1080, height: 1080 },
];

export const FULL_CROP: CropRect = { x: 0, y: 0, width: 1, height: 1 };

export function snapEven(value: number): number {
  const rounded = Math.round(Number.isFinite(value) ? value : 16);
  return Math.max(16, rounded - (rounded % 2));
}

export function validateDimensions(width: number, height: number, maxTextureDimension: number): string | null {
  if (!Number.isFinite(width) || !Number.isFinite(height)) return 'Width and height must be numbers.';
  if (width < 16 || height < 16) return 'Width and height must be at least 16 px.';
  if (width % 2 !== 0 || height % 2 !== 0) return 'H.264 requires even width and height.';
  const limit = Math.max(64, Math.floor(maxTextureDimension));
  if (width > limit || height > limit) {
    return `${width}×${height} exceeds this graphics device’s maximum texture size (${limit} px).`;
  }
  return null;
}

export interface RangeCoverage {
  spanHours: number;
  recordedHours: number;
  missingHours: number;
  gapCount: number;
}

export function summarizeRange(
  stretches: readonly RecordedStretch[],
  unheld: readonly UnheldGap[],
  fromHour: number,
  toHour: number,
): RangeCoverage {
  const lo = Math.min(fromHour, toHour);
  const hi = Math.max(fromHour, toHour);
  const spanHours = Math.max(0, hi - lo);
  let recordedHours = 0;
  for (const stretch of stretches) {
    const a = Math.max(lo, stretch.first);
    const b = Math.min(hi, stretch.last);
    if (b >= a) {
      recordedHours += b - a + 1;
    }
  }
  const totalInclusive = spanHours + 1;
  recordedHours = Math.min(recordedHours, totalInclusive);
  const missingHours = Math.max(0, totalInclusive - recordedHours);
  let gapCount = 0;
  for (const gap of unheld) {
    if (gap.last >= lo && gap.first <= hi) {
      gapCount += 1;
    }
  }
  if (gapCount === 0 && missingHours > 0) {
    gapCount = 1;
  }
  return { spanHours, recordedHours, missingHours, gapCount };
}

export function effectiveExportHours(coverage: RangeCoverage, skipGaps: boolean): number {
  const base = skipGaps ? Math.max(0, coverage.recordedHours - 1) : Math.max(0, coverage.spanHours);
  return base;
}

function spanFrames(first: number, last: number, stepHours: number): number {
  const span = Math.max(0, last - first);
  if (span <= 0) return 1;
  const count = Math.floor(span / stepHours) + 1;
  const lastPos = Math.min(last, first + (count - 1) * stepHours);
  return count + (Math.floor(lastPos) < last ? 1 : 0);
}

export function estimateScheduleFrames(
  stretches: readonly RecordedStretch[],
  fromHour: number,
  toHour: number,
  fps: number,
  hoursPerSecond: number,
  skipGaps: boolean,
  cutCardSeconds: number,
): number {
  const lo = Math.min(fromHour, toHour);
  const hi = Math.max(fromHour, toHour);
  const safeFps = Math.max(1, fps);
  const stepHours = Math.max(0.01, hoursPerSecond) / safeFps;
  const clipped = stretches
    .filter(s => s.last >= lo && s.first <= hi)
    .map(s => ({ first: Math.max(lo, s.first), last: Math.min(hi, s.last) }));
  if (clipped.length === 0) return 0;
  if (!skipGaps) {
    return spanFrames(lo, hi, stepHours);
  }
  const cutFrames = Number.isFinite(cutCardSeconds) && cutCardSeconds > 0
    ? Math.max(1, Math.round(cutCardSeconds * safeFps))
    : 0;
  let total = 0;
  for (let i = 0; i < clipped.length; i += 1) {
    const stretch = clipped[i]!;
    if (i > 0) {
      const prev = clipped[i - 1]!;
      if (prev.last + 1 <= stretch.first - 1) {
        total += cutFrames;
      }
    }
    total += spanFrames(stretch.first, stretch.last, stepHours);
  }
  return total;
}

export function computeDurationSeconds(
  coverage: RangeCoverage,
  hoursPerSecond: number,
  skipGaps: boolean,
  cutCardSeconds: number,
  fps = 60,
  stretches?: readonly RecordedStretch[],
  fromHour?: number,
  toHour?: number,
): number {
  if (stretches && fromHour !== undefined && toHour !== undefined) {
    const frames = estimateScheduleFrames(stretches, fromHour, toHour, fps, hoursPerSecond, skipGaps, cutCardSeconds);
    return frames / Math.max(1, fps);
  }
  const speed = Math.max(0.01, hoursPerSecond);
  const hours = effectiveExportHours(coverage, skipGaps);
  if (hours <= 0) return 1 / Math.max(1, fps);
  const cuts = skipGaps ? coverage.gapCount * Math.max(0, cutCardSeconds) : 0;
  return hours / speed + cuts;
}

export function computeSpeedFromDuration(
  coverage: RangeCoverage,
  durationSeconds: number,
  skipGaps: boolean,
  cutCardSeconds: number,
  fps = 60,
  stretches?: readonly RecordedStretch[],
  fromHour?: number,
  toHour?: number,
): number {
  if (stretches && fromHour !== undefined && toHour !== undefined) {
    const safeFps = Math.max(1, fps);
    const targetFrames = Math.max(1, Math.round(durationSeconds * safeFps));
    const span = Math.max(1, Math.abs(toHour - fromHour));
    let lo = 0.01;
    let hi = Math.max(24, span * safeFps);
    while (estimateScheduleFrames(stretches, fromHour, toHour, safeFps, hi, skipGaps, cutCardSeconds) > targetFrames && hi < 1e9) {
      hi *= 2;
    }
    for (let i = 0; i < 48; i += 1) {
      const mid = (lo + hi) * 0.5;
      const frames = estimateScheduleFrames(stretches, fromHour, toHour, safeFps, mid, skipGaps, cutCardSeconds);
      if (frames > targetFrames) {
        lo = mid;
      } else {
        hi = mid;
      }
    }
    const framesLo = estimateScheduleFrames(stretches, fromHour, toHour, safeFps, lo, skipGaps, cutCardSeconds);
    const framesHi = estimateScheduleFrames(stretches, fromHour, toHour, safeFps, hi, skipGaps, cutCardSeconds);
    const chosen = Math.abs(framesLo - targetFrames) <= Math.abs(framesHi - targetFrames) ? lo : hi;
    return Math.max(0.01, chosen);
  }
  const cuts = skipGaps ? coverage.gapCount * Math.max(0, cutCardSeconds) : 0;
  const videoSeconds = Math.max(0.25, durationSeconds - cuts);
  const hours = Math.max(1 / 60, effectiveExportHours(coverage, skipGaps));
  return Math.max(0.01, hours / videoSeconds);
}

export interface CadenceInfo {
  hoursPerFrame: number;
  framesPerHour: number;
  warning: string | null;
  snappedSpeed: number | null;
}

export function analyzeCadence(hoursPerSecond: number, fps: number): CadenceInfo {
  const safeFps = Math.max(1, fps);
  const hpf = hoursPerSecond / safeFps;
  const fph = hoursPerSecond > 0 ? safeFps / hoursPerSecond : safeFps;
  if (hpf <= 1.0) {
    return { hoursPerFrame: hpf, framesPerHour: fph, warning: null, snappedSpeed: null };
  }
  const nearest = Math.max(1, Math.round(hpf));
  const fractional = Math.abs(hpf - nearest);
  if (fractional > 0.08) {
    return {
      hoursPerFrame: hpf,
      framesPerHour: fph,
      warning: `${hpf.toFixed(2)} hours/frame creates uneven date steps.`,
      snappedSpeed: nearest * safeFps,
    };
  }
  return { hoursPerFrame: hpf, framesPerHour: fph, warning: null, snappedSpeed: null };
}

export function defaultAudioForSpeed(hoursPerSecond: number): boolean {
  return hoursPerSecond <= 24;
}

export function exportGlideDurationMs(hoursPerSecond: number, fps: number): number {
  const safeFps = Math.max(1, fps);
  if (!Number.isFinite(hoursPerSecond) || hoursPerSecond <= 0 || hoursPerSecond >= safeFps) {
    return 0;
  }
  const hourMs = 1000 / hoursPerSecond;
  return Math.max(24, Math.min(180, Math.round(hourMs * 0.85)));
}

export function readabilityHint(minTextPx: number | null, scale: number): string | null {
  if (minTextPx !== null && Number.isFinite(minTextPx) && minTextPx > 0 && minTextPx < 9.5) {
    return `Smallest text renders around ${minTextPx.toFixed(1)} px at this output size. Try Visible viewport, Reflow for portrait, or a higher resolution.`;
  }
  if (Number.isFinite(scale) && scale > 0 && scale < 0.55) {
    return `This page scales to ${Math.round(scale * 100)}% to fit the frame. Inspect the 100% crop preview or switch to Visible viewport.`;
  }
  return null;
}

export function normalizeCrop(crop: CropRect): CropRect {
  const width = Math.min(1, Math.max(0.05, Number.isFinite(crop.width) ? crop.width : 1));
  const height = Math.min(1, Math.max(0.05, Number.isFinite(crop.height) ? crop.height : 1));
  const x = Math.min(1 - width, Math.max(0, Number.isFinite(crop.x) ? crop.x : 0));
  const y = Math.min(1 - height, Math.max(0, Number.isFinite(crop.y) ? crop.y : 0));
  return { x, y, width, height };
}

export function isFullCrop(crop: CropRect): boolean {
  return (
    Math.abs(crop.x) < 1e-4 &&
    Math.abs(crop.y) < 1e-4 &&
    Math.abs(crop.width - 1) < 1e-4 &&
    Math.abs(crop.height - 1) < 1e-4
  );
}

export function fitAspectCrop(sourceViewport: [number, number], targetWidth: number, targetHeight: number): CropRect {
  const [sw, sh] = sourceViewport;
  if (sw <= 0 || sh <= 0 || targetWidth <= 0 || targetHeight <= 0) return FULL_CROP;
  const srcAspect = sw / sh;
  const dstAspect = targetWidth / targetHeight;
  if (Math.abs(srcAspect - dstAspect) < 1e-3) return FULL_CROP;
  if (dstAspect < srcAspect) {
    const width = dstAspect / srcAspect;
    return normalizeCrop({ x: (1 - width) * 0.5, y: 0, width, height: 1 });
  }
  const height = srcAspect / dstAspect;
  return normalizeCrop({ x: 0, y: (1 - height) * 0.5, width: 1, height });
}

export function formatExportDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '0s';
  const total = Math.round(seconds);
  const mins = Math.floor(total / 60);
  const secs = total % 60;
  if (mins >= 60) {
    const hrs = Math.floor(mins / 60);
    const remMins = mins % 60;
    return `${hrs}h ${remMins}m ${secs}s`;
  }
  if (mins > 0) return `${mins}m ${secs}s`;
  return `${secs}s`;
}
