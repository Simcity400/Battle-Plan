import test from 'node:test';
import assert from 'node:assert/strict';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, createBoundedSlice, displayedHour } from '../../families/boundedView.ts';
import type { FamilyTheme } from '../../families/contract.ts';
import { IdentityIndex } from '../../families/identity/index.ts';
import { createSnapshotIdentityProvider } from '../../families/identity/provider.ts';
import { createLaneView } from '../../families/laneRuntime.ts';
import { assertQuietFrames } from '../../families/laneRuntime.fixture.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import type { TimeChartBinding } from '../../families/time-chart/declaration.ts';
import { timeChartLane } from '../../families/time-chart/laneView.ts';
import {
  occupationMixDeclaration,
  occupationMixProjection,
  occupationMixSources,
  type OccupationMixOptions,
} from './occupationMixProjection.ts';

const THEME: FamilyTheme = {
  surface: '#16181d',
  raised: '#1e2128',
  dark: true,
  text: '#eef1f6',
  muted: '#8c95a6',
  line: '#2e3440',
  lineSoft: '#222630',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: '#e5534b',
  shortage: '#ff6b6f',
  amber: '#d29922',
  green: '#3fb950',
  grey: '#8c95a6',
  hatch: '#6e7681',
  hatchEdge: '#484f58',
  font: 'Inter',
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 6,
};

function makeIdentity(hour: number) {
  const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: '1', content: 1 });
  index.ingestStylesReply({
    branchId: '1',
    hour,
    styles: [
      { tag: 'GER', name: 'German Reich', color: [102, 102, 102], valid: [hour - 50, hour + 50], flagChoice: { keys: ['GER_fascism', null, null], reasons: [null, null, null] } },
    ],
    warnings: [],
  });
  index.setIdentities({
    branchId: '1',
    known: [{ firstHour: hour - 50, lastHour: hour + 50 }],
    unknown: [],
    countries: [
      { tag: 'GER', identities: [{ countryId: 7, slot: 3, firstHour: hour - 50, lastHour: hour + 50 }], unresolved: [] },
    ],
    warnings: [],
  });
  return createSnapshotIdentityProvider(index);
}

function mkOptions(overrides: Partial<OccupationMixOptions> = {}): OccupationMixOptions {
  return {
    subject: countryKey(3),
    shown: 'both',
    rank: 'resistance',
    collaboration: false,
    digits: 2,
    endLabels: true,
    extent: [450, 550],
    gaps: [],
    surface: '#16181d',
    ...overrides,
  };
}

test('occupation-trend mix projects solid resistance and dashed compliance lines for one recorded slot', () => {
  const tag = countryKey(3);
  const mkSeries = (minValue: number, maxValue: number) => ({
    tag,
    decimals: 5,
    binHours: 1,
    from: 450,
    buckets: createBoundedSlice([{
      firstHour: 450,
      lastHour: 500,
      firstValue: minValue,
      lastValue: maxValue,
      lastValueHour: 500,
      minValue,
      minHour: 450,
      maxValue,
      maxHour: 500,
      firstBreak: false,
      hasBreak: false,
    }], 1),
    straddling: 'none' as const,
    missing: createBoundedSlice([], 0),
    validInterval: [450, 500] as const,
  });
  const model = occupationMixProjection.project(createBoundedReadView({
    hour: displayedHour(500),
    identity: makeIdentity(500),
    series: new Map([
      ['main', new Map([[tag, mkSeries(18, 24)]])],
      ['second', new Map([[tag, mkSeries(40, 62)]])],
    ]),
  }), mkOptions());

  assert.equal(model.mode, 'lines');
  assert.equal(model.series.length, 2);
  assert.equal(model.series[0]!.name, 'German Reich · Resistance');
  assert.equal(model.series[1]!.name, 'German Reich · Compliance');
  assert.ok(model.series[1]!.dash?.length);
  assert.deepEqual(model.legend?.map(l => l.label), ['Resistance', 'Compliance']);
});

test('source plan keys by recorded slot, adds secondary measure only when both are shown, and withholds while unresolved', () => {
  const mkCtx = (countrySubject: string | null, options: Record<string, unknown> = {}) => ({
    track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
    branch: { id: '1', firstHour: 450, lastHour: 550 },
    countrySubject,
    options,
  }) as unknown as FamilyPlanContext;

  assert.equal(occupationMixSources(mkCtx(null)).entries.length, 0);
  const single = occupationMixSources(mkCtx(countryKey(3), { measure: 'resistance' }));
  assert.deepEqual(single.entries.map(e => e.name), ['main', 'hover', 'detail']);
  const both = occupationMixSources(mkCtx(countryKey(3), { measure: 'both', collaboration: true }));
  assert.deepEqual(both.entries.map(e => e.name), ['main', 'second', 'hover', 'detail']);
  const detailRead = both.entries.find(e => e.name === 'detail')!.read;
  assert.equal(detailRead.shape, 'exact-hover');
  if (detailRead.shape === 'exact-hover') {
    assert.deepEqual(detailRead.params, { branchId: '1', countries: [3], collaboration: true });
  }
  assert.equal(occupationMixDeclaration.countries.mode, 'single');
});

test('a quiet stretch of played hours issues no per-hour panel requests', () => {
  const mkCtx = () => ({
    track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
    branch: { id: '1', firstHour: 450, lastHour: 550 },
    countrySubject: countryKey(3),
    options: { measure: 'both' },
  }) as unknown as FamilyPlanContext;
  const plan = occupationMixSources(mkCtx());
  assert.ok(plan.entries.every(entry => entry.read.shape === 'pixel-binned' || entry.read.shape === 'exact-hover'));
  const binding: TimeChartBinding<OccupationMixOptions> = {
    family: 'time-chart',
    kind: 'occupation-trend',
    view: 'mix',
    options: mkOptions({ shown: 'resistance' }),
    sources: {},
    extent: [450, 550],
    width: 640,
    height: 300,
    scrollTop: 0,
    theme: THEME,
  };
  const next = assertQuietFrames(
    () => createLaneView(timeChartLane, () => {}),
    { label: 'quiet hours', binding, hour: 500 },
    24,
    { width: 640, height: 300, dpr: 1 },
  );
  assert.equal(next, 501);
});
