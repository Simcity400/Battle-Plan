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
  const base = skipGaps ? Math.max(1, coverage.recordedHours - 1) : Math.max(1, coverage.spanHours);
  return Math.max(1 / 60, base);
}

export function computeDurationSeconds(
  coverage: RangeCoverage,
  hoursPerSecond: number,
  skipGaps: boolean,
  cutCardSeconds: number,
): number {
  const speed = Math.max(0.01, hoursPerSecond);
  const hours = effectiveExportHours(coverage, skipGaps);
  const cuts = skipGaps ? coverage.gapCount * Math.max(0, cutCardSeconds) : 0;
  return hours / speed + cuts;
}

export function computeSpeedFromDuration(
  coverage: RangeCoverage,
  durationSeconds: number,
  skipGaps: boolean,
  cutCardSeconds: number,
): number {
  const cuts = skipGaps ? coverage.gapCount * Math.max(0, cutCardSeconds) : 0;
  const videoSeconds = Math.max(0.25, durationSeconds - cuts);
  const hours = effectiveExportHours(coverage, skipGaps);
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
