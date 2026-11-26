import test from 'node:test';
import assert from 'node:assert/strict';
import {
  analyzeCadence,
  computeDurationSeconds,
  computeSpeedFromDuration,
  defaultAudioForSpeed,
  estimateScheduleFrames,
  exportGlideDurationMs,
  fitAspectCrop,
  isFullCrop,
  mergeRecordedTracks,
  mergeUnheldGaps,
  normalizeCrop,
  readabilityHint,
  snapEven,
  summarizeRange,
  validateDimensions,
  widgetShowsRankMarkers,
} from './videoExportModel.ts';
import { easeBarFraction } from '../families/bars/painter.ts';
import { glideDurationForSpeed } from '../families/table/painter.ts';
import { encodeTableSettings, TABLE_SETTINGS_KEY, tableDefaults } from '../families/table/settings.ts';
import { setOption } from '../dashboard/options.ts';
import { TABLE_BASE as GAME_SCORE_TABLE_BASE } from '../widgets/game-score/definition.ts';
import { TABLE_BASE as SCOREBOARD_TABLE_BASE } from '../widgets/scoreboard/definition.ts';
import '../widgets/definitions.ts';
import { widgetOptionFields } from '../widgets/manifest.ts';

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

  const continuousStretches = [{ first: 0, last: 10, join: 'start' as const }];
  const continuousCoverage = summarizeRange(continuousStretches, [], 0, 10);
  const preciseSpeed = computeSpeedFromDuration(continuousCoverage, 95.0, false, 0, 60, continuousStretches, 0, 10);
  const preciseFrames = estimateScheduleFrames(continuousStretches, 0, 10, 60, preciseSpeed, false, 0);
  assert.equal(preciseFrames, 5700);
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

test('easeBarFraction eases smoothly and snaps on missing values or boundaries', () => {
  const mid = easeBarFraction(0.2, 0.8, 0.5);
  assert.ok(mid !== null && mid > 0.6 && mid < 0.8);
  assert.equal(easeBarFraction(0.2, 0.8, 0), 0.2);
  assert.equal(easeBarFraction(0.2, 0.8, 1), 0.8);
  assert.equal(easeBarFraction(null, 0.8, 0.5), 0.8);
  assert.equal(easeBarFraction(undefined, 0.8, 0.5), 0.8);
  assert.equal(easeBarFraction(0.2, null, 0.5), null);
});

test('glideDurationForSpeed honors export override above 168 h/s while keeping live cutoff', () => {
  assert.equal(glideDurationForSpeed(240, false), 0);
  const fastExportMs = exportGlideDurationMs(240, 60);
  assert.ok(fastExportMs > 0, `expected non-zero export glide at 240 h/s, got ${fastExportMs}`);
  assert.equal(glideDurationForSpeed(240, false, fastExportMs), fastExportMs);
  assert.equal(glideDurationForSpeed(240, true, fastExportMs), 0);
  assert.equal(exportGlideDurationMs(24, 60), 35);
  assert.equal(exportGlideDurationMs(1, 60), 180);
});

test('mergeRecordedTracks preserves load joins across overlap and mergeUnheldGaps derives end gaps', () => {
  const merged = mergeRecordedTracks([
    [
      { first: 0, last: 50, join: 'start' },
      { first: 100, last: 150, join: 'break' },
      { first: 151, last: 200, join: 'load' },
    ],
    [{ first: 40, last: 200, join: 'start' }],
  ]);
  assert.equal(merged.length, 2);
  assert.equal(merged[0]!.first, 0);
  assert.equal(merged[0]!.last, 150);
  assert.equal(merged[1]!.first, 151);
  assert.equal(merged[1]!.join, 'load');
  const gaps = mergeUnheldGaps(merged, [[{ first: 51, last: 99, reason: 'track A gap' }], []], 0, 200);
  assert.equal(gaps.length, 0);

  const endGaps = mergeUnheldGaps(
    [{ first: 10, last: 90, join: 'start' }],
    [[
      { first: 0, last: 9, reason: 'leading gap' },
      { first: 91, last: 100, reason: 'trailing gap' },
    ]],
    0,
    100,
  );
  assert.equal(endGaps.length, 2);
  assert.deepEqual(endGaps[0], { first: 0, last: 9, reason: 'leading gap' });
  assert.deepEqual(endGaps[1], { first: 91, last: 100, reason: 'trailing gap' });
});

test('readabilityHint flags small rendered text and steep page downscaling', () => {
  assert.ok(readabilityHint(7.2, 0.8)?.includes('7.2 px'));
  assert.ok(readabilityHint(11.5, 0.4)?.includes('40%'));
  assert.equal(readabilityHint(12, 0.9), null);
});

test('widgetShowsRankMarkers gates extended preview history on effective rank-column visibility', () => {
  assert.equal(widgetShowsRankMarkers(widgetOptionFields('scoreboard', undefined), undefined), false);
  assert.equal(widgetShowsRankMarkers(widgetOptionFields('scoreboard', {}), {}), false);

  const scoreboardWithRanks = setOption(
    widgetOptionFields('scoreboard', undefined),
    undefined,
    TABLE_SETTINGS_KEY,
    encodeTableSettings({ ...tableDefaults([], SCOREBOARD_TABLE_BASE), showRank: true }),
  )!;
  assert.equal(widgetShowsRankMarkers(widgetOptionFields('scoreboard', scoreboardWithRanks), scoreboardWithRanks), true);
  const scoreboardMarkersOff = setOption(
    widgetOptionFields('scoreboard', scoreboardWithRanks),
    scoreboardWithRanks,
    'rankMarkers',
    false,
  )!;
  assert.equal(widgetShowsRankMarkers(widgetOptionFields('scoreboard', scoreboardMarkersOff), scoreboardMarkersOff), false);

  assert.equal(widgetShowsRankMarkers(widgetOptionFields('game-score', undefined), undefined), false);
  const gameScoreWithRanks = setOption(
    widgetOptionFields('game-score', undefined),
    undefined,
    TABLE_SETTINGS_KEY,
    encodeTableSettings({ ...tableDefaults([], GAME_SCORE_TABLE_BASE), showRank: true }),
  )!;
  assert.ok(Object.hasOwn(gameScoreWithRanks, `${TABLE_SETTINGS_KEY}@table`));
  assert.equal(widgetShowsRankMarkers(widgetOptionFields('game-score', gameScoreWithRanks), gameScoreWithRanks), true);
  const gameScoreMarkersOff = setOption(
    widgetOptionFields('game-score', gameScoreWithRanks),
    gameScoreWithRanks,
    'rankMarkers',
    false,
  )!;
  assert.equal(widgetShowsRankMarkers(widgetOptionFields('game-score', gameScoreMarkersOff), gameScoreMarkersOff), false);
  const gameScoreRace = setOption(widgetOptionFields('game-score', gameScoreWithRanks), gameScoreWithRanks, 'view', 'race')!;
  assert.equal(widgetShowsRankMarkers(widgetOptionFields('game-score', gameScoreRace), gameScoreRace), false);
});
