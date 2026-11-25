import assert from 'node:assert/strict';
import test from 'node:test';
import { keyId } from '../../dashboard/keyedSeriesData.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { assertQuietFrames, LANE_THEME } from '../../families/laneRuntime.fixture.ts';
import { createLaneView } from '../../families/laneRuntime.ts';
import { timeChartLane } from '../../families/time-chart/laneView.ts';
import type { TimeChartBinding } from '../../families/time-chart/declaration.ts';
import type { FuelBranchesReply } from '../../generated/dashboard';
import {
  FUEL_BRANCHES_TIME_VIEW,
  fuelBranchesTimeDeclaration,
  fuelBranchesTimeProjection,
  validateFuelParts,
  type FuelBranchesTimeOptions,
} from './fuelBranchesTimeProjection.ts';

const namesReply = (): FuelBranchesReply => ({
  tag: 'JAP',
  scale: 100000,
  consumerNames: ['Army', 'Navy', 'Air Force'],
  consumerIcons: [null, null, null],
  consumerIconReasons: [null, null, null],
  fuelIcon: null,
  fuelIconReason: null,
  readHour: 200,
  missing: null,
  warnings: [],
  consumers: [],
});

const partsReply = () => validateFuelParts({
  branchId: '1',
  partBits: 32,
  owners: [{
    country: 'JAP',
    parts: [
      { part: '2', firstHour: 150, lastHour: 300 },
      { part: '0', firstHour: 100, lastHour: 300 },
      { part: '1', firstHour: 100, lastHour: 300 },
    ],
    missing: [{ firstHour: 250, lastHour: 260, reason: 'Unrecorded gap' }],
  }],
});

const optionsAt = (parts = partsReply()): FuelBranchesTimeOptions => ({
  subject: 'JAP',
  parts,
  endLabels: true,
  extent: [100, 300],
  gaps: [],
  surface: '#161616',
});

test('fuel by branch over time cuts future branches and gaps at the displayed hour, formats exact shares, and projects hover', () => {
  const parts = partsReply();
  assert.deepEqual(parts.parts.map(p => p.part), ['0', '1', '2']);

  const earlyView = createBoundedReadView({
    hour: 120,
    panels: new Map<string, unknown>([
      ['names', namesReply()],
      ['shares', {
        decimals: 4,
        states: [
          { country: 'JAP', part: '0', hour: 120, run: { valueRaw: '1000000' }, missing: null },
          { country: 'JAP', part: '1', hour: 120, run: { valueRaw: '666667' }, missing: null },
        ],
      }],
    ]),
  });
  const early = fuelBranchesTimeProjection.project(earlyView, optionsAt(parts));
  assert.deepEqual(early.series.map(s => s.name), ['Army', 'Navy'], 'future branch Air Force must not leak before hour 150');
  assert.deepEqual(early.series.map(s => s.endLabel?.value), ['100%', '66.66%']);
  assert.equal(early.gaps, undefined, 'future missing span at 250..260 must not leak at hour 120');
  assert.equal(fuelBranchesTimeProjection.nextChange?.(earlyView, optionsAt(parts)), 150);

  const lateView = createBoundedReadView({
    hour: 260,
    panels: new Map<string, unknown>([
      ['names', namesReply()],
      ['shares', {
        decimals: 4,
        states: [
          { country: 'JAP', part: '0', hour: 260, run: { valueRaw: '1000000' }, missing: null },
          { country: 'JAP', part: '1', hour: 260, run: { valueRaw: '500000' }, missing: null },
          { country: 'JAP', part: '2', hour: 260, run: null, missing: 'Not recorded' },
        ],
      }],
    ]),
    hovers: new Map([['hover', {
      hour: 200,
      data: {
        decimals: 4,
        states: [
          { country: 'JAP', part: '0', hour: 200, run: { valueRaw: '1000000' }, missing: null },
          { country: 'JAP', part: '1', hour: 200, run: { valueRaw: '333333' }, missing: null },
          { country: 'JAP', part: '2', hour: 200, run: null, missing: 'Not recorded' },
        ],
      },
    }]]),
  });
  const late = fuelBranchesTimeProjection.project(lateView, optionsAt(parts));
  assert.deepEqual(late.series.map(s => s.name), ['Army', 'Navy', 'Air Force']);
  assert.equal(late.series[2]!.endLabel, null, 'missing head omits end label instead of inventing zero');
  assert.equal(late.gaps?.length, 1);
  assert.deepEqual(late.hoverDetail?.items.map(i => [i.label, i.value ?? i.missingReason]), [
    ['Army', '100%'],
    ['Navy', '33.33%'],
    ['Air Force', 'Not recorded'],
  ]);
  assert.equal(fuelBranchesTimeDeclaration.kind, 'fuel-branches');
});

test('fuel by branch over time stays quiet across unchanged playback hours', () => {
  const parts = partsReply();
  const binding: TimeChartBinding<FuelBranchesTimeOptions> = {
    family: 'time-chart',
    kind: 'fuel-branches',
    view: FUEL_BRANCHES_TIME_VIEW,
    options: optionsAt(parts),
    sources: {},
    staticPanels: {
      names: namesReply(),
      shares: {
        decimals: 4,
        states: [
          { country: 'JAP', part: '0', hour: 200, run: { valueRaw: '1000000' }, missing: null },
          { country: 'JAP', part: '1', hour: 200, run: { valueRaw: '500000' }, missing: null },
          { country: 'JAP', part: '2', hour: 200, run: { valueRaw: '750000' }, missing: null },
        ],
      },
    },
    extent: [100, 300],
    width: 640,
    height: 320,
    scrollTop: 0,
    theme: LANE_THEME,
    identitySnapshot: null,
    selectedKey: null,
    focusedKey: null,
  };
  const surface = { width: 640, height: 320, dpr: 1 };
  assertQuietFrames(
    () => createLaneView(timeChartLane, () => {}),
    { label: 'fuel-branches-time quiet', binding, hour: 200 },
    20,
    surface,
  );
  assert.equal(keyId({ tag: 'JAP', part: '0' }), 'JAP\u00000');
});
