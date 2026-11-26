import test from 'node:test';
import assert from 'node:assert/strict';
import {
  analyzeCadence,
  computeDurationSeconds,
  computeSpeedFromDuration,
  defaultAudioForSpeed,
  estimateScheduleFrames,
  fitAspectCrop,
  isFullCrop,
  normalizeCrop,
  snapEven,
  summarizeRange,
  validateDimensions,
} from './videoExportModel.ts';

test('snapEven and validateDimensions enforce even bounds and GPU texture limits', () => {
  assert.equal(snapEven(1921), 1920);
  assert.equal(snapEven(9), 16);
  assert.equal(validateDimensions(1920, 1080, 8192), null);
  assert.match(validateDimensions(1921, 1080, 8192) ?? '', /even/i);
  assert.match(validateDimensions(10000, 1080, 8192) ?? '', /8192/);
});

test('duration and speed stay linked across arbitrarily high timelapse speeds', () => {
  const coverage = summarizeRange(
    [
      { first: 0, last: 2000, join: 'start' },
      { first: 2500, last: 5000, join: 'break' },
    ],
    [{ first: 2001, last: 2499, reason: 'recorder stopped' }],
    0,
    5000,
  );
  assert.equal(coverage.spanHours, 5000);
  assert.equal(coverage.gapCount, 1);
  assert.equal(coverage.missingHours, 499);

  const fastSpeed = computeSpeedFromDuration(coverage, 2, false, 1);
  assert.ok(fastSpeed > 2400, `expected > 2400 h/s without 1440 cap, got ${fastSpeed}`);
  const roundTripDuration = computeDurationSeconds(coverage, fastSpeed, false, 1);
  assert.ok(Math.abs(roundTripDuration - 2) < 1e-6);

  const skippedDuration = computeDurationSeconds(coverage, 1000, true, 1);
  assert.ok(skippedDuration < computeDurationSeconds(coverage, 1000, false, 1) + 1.0);
});

test('analyzeCadence flags fractional high-speed steps and suggests integer snap', () => {
  assert.equal(analyzeCadence(24, 60).warning, null);
  const uneven = analyzeCadence(150, 60);
  assert.ok(uneven.warning !== null);
  assert.equal(uneven.snappedSpeed, 180);
  assert.equal(analyzeCadence(180, 60).warning, null);
});

test('defaultAudioForSpeed enables near real-time and disables fast timelapses', () => {
  assert.equal(defaultAudioForSpeed(12), true);
  assert.equal(defaultAudioForSpeed(24), true);
  assert.equal(defaultAudioForSpeed(72), false);
});

test('estimateScheduleFrames matches encoder single-hour and multi-stretch cut rules', () => {
  const stretches = [
    { first: 100, last: 199, join: 'start' as const },
    { first: 250, last: 299, join: 'break' as const },
  ];
  assert.equal(estimateScheduleFrames(stretches, 150, 150, 60, 0.1, false, 1.0), 1);
  assert.equal(estimateScheduleFrames(stretches, 150, 150, 60, 0.1, true, 1.0), 1);
  const withCuts = estimateScheduleFrames(stretches, 100, 299, 30, 30, true, 1.0);
  assert.equal(withCuts, 100 + 30 + 50);

  const leadingGapStretches = [{ first: 10, last: 20, join: 'start' as const }];
  const leadingCoverage = summarizeRange(leadingGapStretches, [{ first: 5, last: 9, reason: 'gap' }], 5, 20);
  const solvedSpeed = computeSpeedFromDuration(leadingCoverage, 2.0, true, 1.0, 60, leadingGapStretches, 5, 20);
  const solvedDuration = computeDurationSeconds(leadingCoverage, solvedSpeed, true, 1.0, 60, leadingGapStretches, 5, 20);
  assert.ok(Math.abs(solvedDuration - 2.0) < 0.05, `expected ~2.0s, got ${solvedDuration}s at ${solvedSpeed} h/s`);
});

test('fitAspectCrop and normalizeCrop keep crop rects valid and centered', () => {
  const vertical = fitAspectCrop([1920, 1080], 1080, 1920);
  assert.equal(isFullCrop(vertical), false);
  assert.ok(Math.abs(vertical.height - 1) < 1e-4);
  assert.ok(Math.abs(vertical.x + vertical.width * 0.5 - 0.5) < 1e-4);

  const clamped = normalizeCrop({ x: 0.9, y: -0.2, width: 0.5, height: 0.4 });
  assert.ok(Math.abs(clamped.x - 0.5) < 1e-5);
  assert.equal(clamped.y, 0);
});
