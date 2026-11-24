import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import { deriveTimeChartTooltip } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import { timeChartLaneView } from '../../families/time-chart/laneView.ts';
import { LANE_THEME } from '../../families/laneRuntime.fixture.ts';
import type { OccupationReserveReply, ReserveHistoryReply } from '../../generated/dashboard';
import { FAMILY_VIEWS } from '../familyManifest.ts';
import { occupationReserveBreakdownProjection } from './occupationReserveBreakdownProjection.ts';
import { occupationReserveRaceProjection } from './occupationReserveRaceProjection.ts';

function fakeIdentity(names: Record<string, { name: string; color: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = kind === 'country' ? names[key] : undefined;
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: hit ? { status: 'ready', value: hit.name } : { status: 'absent', reason: 'Name not recorded' },
        colour: hit ? { status: 'ready', value: hit.color } : { status: 'absent', reason: 'Colour not recorded' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

test('occupation-reserve-manpower race preserves Rust ranking, builds source-country segments, and separates negative entries in tooltip', () => {
  const reply: OccupationReserveReply = {
    holders: [
      {
        country: { id: 2, tag: 'GER' },
        admitted: true,
        total: 500,
        entries: [
          { ordinal: 0, source: { id: 8, tag: 'ITA' }, amount: -50 },
          { ordinal: 1, source: { id: 9, tag: 'HUN' }, amount: 300 },
          { ordinal: 2, source: { id: 9, tag: 'HUN' }, amount: 200 },
        ],
      },
      {
        country: { id: 3, tag: 'ENG' },
        admitted: true,
        total: 250,
        entries: [
          { ordinal: 0, source: { id: 10, tag: null }, amount: 250 },
        ],
      },
    ],
    readHour: 240,
    unnamed: 1,
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(240),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      GER: { name: 'German Reich', color: '#6b7280' },
      ENG: { name: 'United Kingdom', color: '#e11d48' },
      ITA: { name: 'Italy', color: '#22c55e' },
      HUN: { name: 'Hungary', color: '#f59e0b' },
    }),
  });
  const model = occupationReserveRaceProjection.project(view, { top: 10, all: false });
  assertBarsModelValid(model, 240);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.numeric]), [['GER', 500], ['ENG', 250]]);
  assert.deepEqual(rows[0]!.segments?.map(s => [s.key, s.share]), [['HUN', 1]]);
  assert.ok(rows[0]!.tooltip?.items?.some(item => item.label === 'Negative entries'));
  assert.ok(model.warnings?.some(w => w.includes('1 holders')));
});

test('occupation-reserve-manpower surfaces missing refusal as unavailable and handles unpopulated default Top cleanly', () => {
  const unpopulated = createBoundedReadView({ hour: displayedHour(240), panels: new Map() });
  const pending = occupationReserveRaceProjection.project(unpopulated, { top: 10, all: false });
  assert.equal(pending.rows.length, 0);
  assert.equal(pending.empty, undefined);

  const missing: OccupationReserveReply = {
    holders: [],
    readHour: null,
    unnamed: 0,
    missing: 'Occupation reserve was not recorded at this hour',
    warnings: [],
  };
  const refused = occupationReserveRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(240), panels: new Map([['main', missing]]) }),
    { top: 10, all: false },
  );
  assert.equal(refused.emptyKind, 'unavailable');
  assert.equal(refused.empty, missing.missing);
});

test('occupation-reserve-manpower breakdown projects causal stacked step bands, gaps, exact dated hover, and negative notes via manifest time-chart entry', async () => {
  const entry = FAMILY_VIEWS.find(item => item.kind === 'occupation-reserve-manpower' && item.view === 'breakdown');
  assert.equal(entry?.family, 'time-chart');
  assert.equal(entry?.viewOption, 'show');
  assert.deepEqual(entry?.options?.map(field => field.key), ['holder', 'show', 'yMin', 'yMax']);
  const spec = await entry!.loadSpec();
  assert.equal(spec.projection, occupationReserveBreakdownProjection);

  const history: ReserveHistoryReply = {
    segments: [
      { firstHour: 100, lastHour: 199, admitted: true, entries: [{ source: 9, amount: 300 }, { source: 8, amount: 100 }], unknown: null },
      { firstHour: 200, lastHour: 249, admitted: false, entries: null, unknown: 'The reserve was not recorded at these hours' },
      { firstHour: 250, lastHour: 350, admitted: true, entries: [{ source: 9, amount: 400 }, { source: 8, amount: -50 }], unknown: null },
      { firstHour: 351, lastHour: 450, admitted: true, entries: [{ source: 11, amount: 900 }], unknown: null },
    ],
    sources: [
      { id: 8, tags: [{ tag: 'ITA', firstHour: 100, lastHour: 450 }] },
      { id: 9, tags: [{ tag: 'HUN', firstHour: 100, lastHour: 220 }, { tag: 'AUH', firstHour: 221, lastHour: 450 }] },
      { id: 11, tags: [{ tag: 'ROM', firstHour: 351, lastHour: 450 }] },
    ],
    warnings: [],
  };

  const datedIdentity: IdentityProvider = {
    resolve(kind, key, atHour) {
      const name = key === 'GER' ? 'German Reich'
        : key === 'ITA' ? 'Italy'
          : key === 'HUN' ? (atHour <= 220 ? 'Kingdom of Hungary' : 'Hungary')
            : key === 'AUH' ? 'Austria-Hungary'
              : null;
      const color = key === 'ITA' ? '#22c55e' : key === 'HUN' || key === 'AUH' ? '#f59e0b' : '#6b7280';
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: name ? { status: 'ready', value: name } : { status: 'absent', reason: 'Name not recorded' },
        colour: { status: 'ready', value: color },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };

  const hoverReply: OccupationReserveReply = {
    holders: [{
      country: { id: 2, tag: 'GER' },
      admitted: true,
      total: 300,
      entries: [
        { ordinal: 0, source: { id: 9, tag: 'HUN' }, amount: 300 },
        { ordinal: 1, source: { id: 8, tag: 'ITA' }, amount: -25 },
      ],
    }],
    readHour: 150,
    unnamed: 0,
    missing: null,
    warnings: [],
  };

  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map<string, unknown>([['history', history]]),
    hovers: new Map([['hover', { hour: 150, data: hoverReply }]]),
    identity: datedIdentity,
  });

  const model = occupationReserveBreakdownProjection.project(view, {
    holder: { tag: 'GER', id: 2 },
    extent: [100, 450],
    gaps: [],
    surface: '#161616',
    yMin: 0,
    yMax: 1000,
  });

  assert.equal(model.mode, 'stack');
  assert.equal(model.stack?.step, true);
  assert.deepEqual([model.yAxis.min, model.yAxis.max], [0, 1000]);
  assert.deepEqual(model.stack?.bands.map(b => b.id), ['9', '8'], 'future source 11 after hour 300 is excluded');
  assert.equal(model.stack?.bands[0]!.name, 'Austria-Hungary', 'band name reflects drawn hour identity');
  assert.deepEqual(model.stack?.hours, [100, 200, 250]);
  assert.deepEqual(model.stack?.bands[0]!.values, [300, null, 400]);
  assert.deepEqual(model.stack?.bands[1]!.values, [100, null, 0]);
  assert.ok(model.gaps?.some(g => g.firstHour === 200 && g.lastHour === 249));
  assert.equal(model.note, 'Negative entries: Italy −50');
  assert.deepEqual(model.auditRows?.map(r => [r.key, r.value]), [['__total__', '400'], ['9', '400']]);

  const layout = layoutTimeChart(model, 600, 260, 'sans-serif', t => t.length * 6, 150, null);
  const tip = deriveTimeChartTooltip(model, layout, 'plot:150');
  assert.ok(tip?.payload?.items?.some(item => item.label === 'Kingdom of Hungary' && item.value === '300'), 'hover resolves source name at hovered hour');
  assert.ok(tip?.payload?.items?.some(item => item.label === 'Negative entries'));
  assert.ok(tip?.payload?.items?.some(item => item.label === 'Italy' && item.value === '−25'));

  const lane = timeChartLaneView(() => {});
  assert.ok(lane);
  const unchosen = occupationReserveBreakdownProjection.project(view, {
    holder: null,
    extent: [100, 450],
    gaps: [],
    surface: '#161616',
    yMin: null,
    yMax: null,
  });
  assert.equal(unchosen.empty?.message, 'Choose a holder: click its bar in the race.');
  assert.equal(LANE_THEME.surface.length > 0, true);
});
