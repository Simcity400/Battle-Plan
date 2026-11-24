import assert from 'node:assert/strict';
import test from 'node:test';
import type { ReachedLine } from '../../widgets/country-measure/measureSeriesData.ts';
import { POLITICAL_POWER_DECLARATION } from './declaration.ts';
import { buildMarkTooltip, buildMeasureTooltip, formatLegendReading } from './tooltipFormat.ts';
import { ClosedTilesCache, unrecordedFromTransfers } from './tilesClient.ts';
import type { DecodedTile } from './tiles.ts';

test('political-power declaration uses powerAxis and spend ticks', () => {
  assert.equal(POLITICAL_POWER_DECLARATION.kind, 'political-power');
  assert.equal(POLITICAL_POWER_DECLARATION.family, 'country-measure');
  assert.equal(POLITICAL_POWER_DECLARATION.measure, 'politicalPower');
  assert.equal(POLITICAL_POWER_DECLARATION.formatValue, 'powerAxis');
  assert.equal(POLITICAL_POWER_DECLARATION.spendTicks, true);
  assert.equal(POLITICAL_POWER_DECLARATION.unit, 'PP');
});

test('tooltip distinguishes pending, notApplicable, not recorded, and recorded values with drop', () => {
  const lineByTag = new Map<string, ReachedLine>([
    ['GER', {
      tag: 'GER',
      runs: [{
        firstHour: 100,
        lastHour: 200,
        valueRaw: '123456',
        startsSegment: true,
        slot: 1,
        countryId: 1,
        segmentFirstHour: 100,
        firstExact: true,
        lastExact: true,
      }],
      drops: [{ hour: 150, beforeRaw: '173456', afterRaw: '123456' }],
      missing: new Map(),
      notApplicable: new Set(),
    }],
    ['ENG', {
      tag: 'ENG',
      runs: [],
      drops: [],
      missing: new Map([[150, 'Not applicable for this holder']]),
      notApplicable: new Set([150]),
    }],
    ['FRA', {
      tag: 'FRA',
      runs: [],
      drops: [],
      missing: new Map([[150, 'Table truncated']]),
      notApplicable: new Set(),
    }],
    ['SOV', {
      tag: 'SOV',
      runs: [],
      drops: [],
      missing: new Map(),
      notApplicable: new Set(),
    }],
  ]);

  const tip = buildMeasureTooltip({
    hour: 150,
    series: [
      { tag: 'GER', hex: '#111111' },
      { tag: 'ENG', hex: '#222222' },
      { tag: 'FRA', hex: '#333333' },
      { tag: 'SOV', hex: '#444444' },
    ],
    lineByTag,
    hourLoaded: (tag, h) => tag !== 'SOV' && h === 150,
    labelAt: () => tag => ({ name: tag }),
    decimals: 3,
    digits: 0,
    unit: 'PP',
    badges: [],
    viewportHeight: 800,
  });

  const rows = tip.rows ?? [];
  assert.equal(rows[0]!.value, '123 PP');
  assert.equal(rows[1]!.value, 'Not applicable');
  assert.equal(rows[1]!.tone, 'muted');
  assert.equal(rows[1]!.detail, 'Not applicable for this holder');
  assert.equal(rows[2]!.value, 'Not recorded');
  assert.equal(rows[2]!.tone, 'muted');
  assert.equal(rows[2]!.detail, 'Table truncated');
  assert.equal(rows[3]!.value, 'Loading balance…');
  assert.equal(rows[3]!.tone, 'muted');
  assert.equal(rows[4]!.label, 'GER: balance fell');
  assert.equal(rows[4]!.value, '−50');
});

test('legend reading formatter distinguishes pending, notApplicable, unrecorded, and value', () => {
  assert.equal(formatLegendReading(undefined, 3, 0, false), 'Loading…');
  assert.equal(formatLegendReading(undefined, 3, 0, true), 'Not recorded');
  assert.equal(formatLegendReading({ valueRaw: null, value: null, missing: null, notApplicable: true }, 3, 0, true), 'Not applicable');
  assert.equal(formatLegendReading({ valueRaw: null, value: null, missing: 'Truncated', notApplicable: false }, 3, 0, true), 'Not recorded');
  assert.equal(formatLegendReading({ valueRaw: '45000', value: 45, missing: null, notApplicable: false }, 3, 0, true), '45');
});

test('mark tooltip formats cluster count and drop spend', () => {
  const lineByTag = new Map<string, ReachedLine>([
    ['GER', {
      tag: 'GER',
      runs: [],
      drops: [{ hour: 240, beforeRaw: '200000', afterRaw: '150000' }],
      missing: new Map(),
      notApplicable: new Set(),
    }],
  ]);
  const tip = buildMarkTooltip({
    cluster: { firstHour: 240, lastHour: 240, count: '2', firstOrdinal: '10', tag: 'GER' },
    lineByTag,
    labelAt: () => tag => ({ name: tag }),
    decimals: 3,
    digits: 0,
  });
  assert.equal(tip.rows?.[0]?.value, '2');
  assert.equal(tip.rows?.[1]?.label, 'Balance fell');
  assert.equal(tip.rows?.[1]?.value, '−50');
});

test('unrecordedFromTransfers returns reason only when every bin is empty with one reason', () => {
  const emptyTile = (reason: string, hasPresent = false): DecodedTile => ({
    key: 'k',
    identity: {
      sourceId: 's',
      revision: 'r',
      branchId: 'b',
      kind: 'country-measure',
      measure: 'politicalPower',
      role: 'primary',
      country: 'GER',
      binHours: 24,
      index: 0,
      withMarks: false,
    },
    seriesKey: 'primary\0GER',
    binHours: 24,
    index: 0,
    firstHour: 0,
    lastHour: 24 * 256 - 1,
    immutable: true,
    filled: 100,
    decimals: 3,
    present: new Uint8Array(256).fill(hasPresent ? 1 : 0),
    flags: new Uint8Array(256),
    firstHours: new Int32Array(256),
    lastHours: new Int32Array(256),
    lastValueHours: new Int32Array(256),
    minHours: new Int32Array(256),
    maxHours: new Int32Array(256),
    firstValues: new Float64Array(256),
    lastValues: new Float64Array(256),
    minValues: new Float64Array(256),
    maxValues: new Float64Array(256),
    missing: [{ firstHour: 0, lastHour: 100, reason }],
    parts: [],
    marksBefore: null,
    drops: [],
    bytes: 1024,
  });

  assert.equal(
    unrecordedFromTransfers([{ key: 'primary\0GER', binHours: 24, firstIndex: 0, lastIndex: 0, tiles: [emptyTile('Table absent')] }]),
    'Table absent',
  );
  assert.equal(
    unrecordedFromTransfers([{ key: 'primary\0GER', binHours: 24, firstIndex: 0, lastIndex: 0, tiles: [emptyTile('Table absent', true)] }]),
    null,
  );
});

test('ClosedTilesCache signature stays stable when lastTile grows into an uncached future tile', () => {
  const cache = new ClosedTilesCache();
  const baseTile: DecodedTile = {
    key: 's|r|b|country-measure|politicalPower|primary|GER|24|0|0',
    identity: {
      sourceId: 's',
      revision: 'r',
      branchId: 'b',
      kind: 'country-measure',
      measure: 'politicalPower',
      role: 'primary',
      country: 'GER',
      binHours: 24,
      index: 0,
      withMarks: false,
    },
    seriesKey: 'primary\0GER',
    binHours: 24,
    index: 0,
    firstHour: 0,
    lastHour: 24 * 256 - 1,
    immutable: true,
    filled: 24 * 256 - 1,
    decimals: 3,
    present: new Uint8Array(256).fill(1),
    flags: new Uint8Array(256),
    firstHours: new Int32Array(256),
    lastHours: new Int32Array(256),
    lastValueHours: new Int32Array(256),
    minHours: new Int32Array(256),
    maxHours: new Int32Array(256),
    firstValues: new Float64Array(256),
    lastValues: new Float64Array(256),
    minValues: new Float64Array(256),
    maxValues: new Float64Array(256),
    missing: [],
    parts: [],
    marksBefore: null,
    drops: [],
    bytes: 1024,
  };
  cache.set(baseTile);
  const req = {
    source: { id: 's', revision: 'r' },
    branchId: 'b',
    kind: 'country-measure',
    measure: 'politicalPower',
    role: 'primary',
    tags: ['GER'],
    binHours: 24,
    firstTile: 0,
    lastTile: 0,
    lastHour: 24 * 512,
    withMarks: false,
    span: () => null,
  };
  const first = cache.collect(req);
  const ahead = cache.collect({ ...req, lastTile: 1 });
  assert.equal(ahead.signature, first.signature);
  assert.equal(ahead.missing.has(1), true);
});
