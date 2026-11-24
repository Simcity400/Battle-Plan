import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { BranchToHourSourceRead, ExactHoverSourceRead, FamilyPlanContext, IdentityProvider } from '../../families/plan.ts';
import { deriveTimeChartTooltip } from '../../families/time-chart/hitTest.ts';
import { hoverTimeChartLayout, layoutTimeChart } from '../../families/time-chart/layout.ts';
import { timeChartLane } from '../../families/time-chart/laneView.ts';
import { LANE_THEME } from '../../families/laneRuntime.fixture.ts';
import { createLaneView } from '../../families/laneRuntime.ts';
import type { CountryIdentitiesReply, OccupationReserveReply, ReserveHistoryReply } from '../../generated/dashboard';
import { FAMILY_VIEWS } from '../familyManifest.ts';
import { occupationReserveBreakdownDeclaration, occupationReserveBreakdownProjection } from './occupationReserveBreakdownProjection.ts';
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
        country: { id: 2, tag: 'AAA' },
        slot: null,
        admitted: true,
        total: 500,
        entries: [
          { ordinal: 0, source: { id: 8, tag: 'BBB' }, amount: -50 },
          { ordinal: 1, source: { id: 9, tag: 'CCC' }, amount: 300 },
          { ordinal: 2, source: { id: 9, tag: 'CCC' }, amount: 200 },
        ],
      },
      {
        country: { id: 3, tag: 'DDD' },
        slot: null,
        admitted: true,
        total: 250,
        entries: [
          { ordinal: 0, source: { id: 10, tag: null }, amount: 250 },
        ],
      },
    ],
    readHour: 240,
    unnamed: 1,
    noneMissing: null,
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(240),
    panels: new Map([['main', reply]]),
    identity: fakeIdentity({
      AAA: { name: 'Alpha', color: '#6b7280' },
      DDD: { name: 'Delta', color: '#e11d48' },
      BBB: { name: 'Beta', color: '#22c55e' },
      CCC: { name: 'Gamma', color: '#f59e0b' },
    }),
  });
  const model = occupationReserveRaceProjection.project(view, { top: 10, all: false, group: false });
  assertBarsModelValid(model, 240);
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.numeric]), [['AAA', 500], ['DDD', 250]]);
  assert.deepEqual(rows[0]!.segments?.map(s => [s.key, s.share]), [['CCC', 1]]);
  assert.ok(rows[0]!.tooltip?.items?.some(item => item.label === 'Negative entries'));
  assert.ok(model.warnings?.some(w => w.includes('1 holders')));
});

test('occupation-reserve-manpower surfaces missing refusal as unavailable and handles unpopulated default Top cleanly', () => {
  const unpopulated = createBoundedReadView({ hour: displayedHour(240), panels: new Map() });
  const pending = occupationReserveRaceProjection.project(unpopulated, { top: 10, all: false, group: false });
  assert.equal(pending.rows.length, 0);
  assert.equal(pending.empty, undefined);

  const missing: OccupationReserveReply = {
    holders: [],
    readHour: null,
    unnamed: 0,
    noneMissing: null,
    missing: 'Occupation reserve was not recorded at this hour',
    warnings: [],
  };
  const refused = occupationReserveRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(240), panels: new Map([['main', missing]]) }),
    { top: 10, all: false, group: false },
  );
  assert.equal(refused.emptyKind, 'unavailable');
  assert.equal(refused.empty, missing.missing);
  const unproven = occupationReserveRaceProjection.project(
    createBoundedReadView({ hour: displayedHour(240), panels: new Map([['main', { ...missing, missing: null, unnamed: 2, noneMissing: '2 holders the recording names no tag for hold reserve manpower at this hour' }]]) }),
    { top: 10, all: false, group: false },
  );
  assert.deepEqual([unproven.emptyKind, unproven.empty], ['unavailable', '2 holders the recording names no tag for hold reserve manpower at this hour'], 'an empty reserve is claimed only with the evidence for it');
});

test('occupation-reserve-manpower breakdown asks history to the hour, asks hover by recorded slot, and rejects future or foreign-holder leaks', async () => {
  const entry = FAMILY_VIEWS.find(item => item.kind === 'occupation-reserve-manpower' && item.view === 'breakdown');
  assert.equal(entry?.family, 'time-chart');
  assert.equal(entry?.viewOption, 'show');
  assert.deepEqual(entry?.options?.map(field => field.key), ['holder', 'show', 'yMin', 'yMax']);
  const spec = await entry!.loadSpec();
  assert.equal(spec.projection, occupationReserveBreakdownProjection);

  const aux: CountryIdentitiesReply = {
    branchId: '1',
    known: [],
    unknown: [],
    countries: [
      { tag: 'AAA', identities: [{ countryId: 2, slot: 7, firstHour: 100, lastHour: 450 }], unresolved: [] },
    ],
    warnings: [],
  };
  const ctx = {
    track: { source: { id: 's1', revision: 'r1' }, axisOffset: 0 },
    branch: { id: '1', firstHour: 100, lastHour: 450 },
    options: { holder: 'AAA|2' },
    countries: { all: false, top: null, tags: ['AAA'], holders: null },
    aux,
  } as unknown as FamilyPlanContext<CountryIdentitiesReply>;
  const plan = occupationReserveBreakdownDeclaration.sources(ctx, { width: 600, height: 260, following: false, staticSpan: [100, 450] });
  const histRead = plan.entries.find(e => e.name === 'history')?.read as BranchToHourSourceRead;
  const hoverRead = plan.entries.find(e => e.name === 'hover')?.read as ExactHoverSourceRead;
  assert.deepEqual(
    histRead.buildSpanParams!(histRead.params!, { firstHour: 100, lastHour: 300 } as Parameters<NonNullable<BranchToHourSourceRead['buildSpanParams']>>[1]),
    { branchId: '1', holder: 2, hour: 300 },
  );
  assert.deepEqual(histRead.dynamicKeys, ['hour']);
  assert.deepEqual(hoverRead.params, { branchId: '1', countries: [7] }, 'exact hover asks by recorded slot rather than reused tag');

  const history: ReserveHistoryReply = {
    hour: 300,
    nextChange: 351,
    segments: [
      { firstHour: 100, lastHour: 199, admitted: true, entries: [{ source: 9, amount: 300 }, { source: 8, amount: 100 }], unknown: null },
      { firstHour: 200, lastHour: 249, admitted: false, entries: null, unknown: 'The reserve was not recorded at these hours' },
      { firstHour: 250, lastHour: 300, admitted: true, entries: [{ source: 9, amount: 400 }, { source: 8, amount: -50 }, { source: 12, amount: 50 }], unknown: null },
    ],
    sources: [
      { id: 8, tags: [{ tag: 'BBB', firstHour: 100, lastHour: 300 }] },
      { id: 9, tags: [{ tag: 'CCC', firstHour: 100, lastHour: 220 }, { tag: 'CCC2', firstHour: 221, lastHour: 300 }] },
      { id: 12, tags: [{ tag: null, firstHour: 250, lastHour: 300 }] },
    ],
    warnings: [],
  };

  const datedIdentity: IdentityProvider = {
    resolve(kind, key, atHour) {
      const name = key === 'AAA' ? 'Alpha'
        : key === 'BBB' ? 'Beta'
          : key === 'CCC' ? (atHour <= 220 ? 'Old Gamma' : 'Gamma')
            : key === 'CCC2' ? 'New Gamma'
              : null;
      const color = key === 'BBB' ? '#22c55e' : key === 'CCC' || key === 'CCC2' ? '#f59e0b' : '#6b7280';
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
      country: { id: 2, tag: 'AAA' },
      slot: null,
      admitted: true,
      total: 300,
      entries: [
        { ordinal: 0, source: { id: 9, tag: 'CCC' }, amount: 300 },
        { ordinal: 1, source: { id: 8, tag: 'BBB' }, amount: -25 },
      ],
    }],
    readHour: 150,
    unnamed: 0,
    noneMissing: null,
    missing: null,
    warnings: [],
  };

  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map<string, unknown>([['history', history]]),
    hovers: new Map([['hover', { hour: 150, data: hoverReply }]]),
    identity: datedIdentity,
  });

  assert.equal(occupationReserveBreakdownProjection.nextChange?.(view, {
    holder: { tag: 'AAA', id: 2, slot: 7 },
    extent: [100, 450],
    gaps: [],
    surface: '#161616',
    yMin: 0,
    yMax: 1000,
  }), 351);

  const model = occupationReserveBreakdownProjection.project(view, {
    holder: { tag: 'AAA', id: 2, slot: 7 },
    extent: [100, 450],
    gaps: [],
    surface: '#161616',
    yMin: 0,
    yMax: 1000,
  });

  assert.equal(model.mode, 'stack');
  assert.equal(model.stack?.step, true);
  assert.deepEqual([model.yAxis.min, model.yAxis.max], [0, 1000]);
  assert.deepEqual(model.stack?.bands.map(b => b.id), ['9', '8', '12']);
  assert.equal(model.stack?.bands[0]!.name, 'New Gamma', 'band name reflects drawn hour identity');
  assert.equal(model.stack?.bands[2]!.name, 'Source #12 (tag not recorded)', 'unresolved source keeps recorded key and reason');
  assert.deepEqual(model.stack?.hours, [100, 200, 250]);
  assert.deepEqual(model.stack?.bands[0]!.values, [300, null, 400]);
  assert.deepEqual(model.stack?.bands[1]!.values, [100, null, 0]);
  assert.deepEqual(model.stack?.bands[2]!.values, [0, null, 50]);
  assert.ok(model.gaps?.some(g => g.firstHour === 200 && g.lastHour === 249));
  assert.equal(model.note, 'Negative entries: Beta −50');
  assert.deepEqual(model.auditRows?.map(r => [r.key, r.value]), [['__total__', '450'], ['9', '400'], ['12', '50']]);

  const layout = hoverTimeChartLayout(layoutTimeChart(model, 600, 260, 'sans-serif', t => t.length * 6), null);
  const tip = deriveTimeChartTooltip(model, layout, 'plot:150');
  assert.ok(tip?.payload?.items?.some(item => item.label === 'Old Gamma' && item.value === '300'), 'hover resolves source name at hovered hour');
  assert.ok(tip?.payload?.items?.some(item => item.label === 'Negative entries'));
  assert.ok(tip?.payload?.items?.some(item => item.label === 'Beta' && item.value === '−25'));

  const foreignHolderHover: OccupationReserveReply = {
    holders: [{ country: { id: 99, tag: 'AAA' }, slot: null, admitted: true, total: 999, entries: [{ ordinal: 0, source: { id: 8, tag: 'BBB' }, amount: 999 }] }],
    readHour: 220,
    unnamed: 0,
    noneMissing: null,
    missing: null,
    warnings: [],
  };
  const unreadHoverView = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map<string, unknown>([['history', history]]),
    hovers: new Map([['hover', { hour: 220, data: foreignHolderHover }]]),
    identity: datedIdentity,
  });
  const unreadHoverModel = occupationReserveBreakdownProjection.project(unreadHoverView, {
    holder: { tag: 'AAA', id: 2, slot: 7 },
    extent: [100, 450],
    gaps: [],
    surface: '#161616',
    yMin: null,
    yMax: null,
  });
  assert.equal(unreadHoverModel.hoverDetail?.items[0]?.missingReason, 'The reserve was not recorded at these hours', 'hover never falls back to a different country id and preserves unread coverage');

  const unreadHeadHistory: ReserveHistoryReply = {
    hour: 220,
    nextChange: 250,
    segments: [
      { firstHour: 100, lastHour: 199, admitted: true, entries: [], unknown: null },
      { firstHour: 200, lastHour: 220, admitted: false, entries: null, unknown: 'Occupation settings were not recorded at these hours' },
    ],
    sources: [],
    warnings: [],
  };
  const unreadHeadModel = occupationReserveBreakdownProjection.project(
    createBoundedReadView({ hour: displayedHour(220), panels: new Map<string, unknown>([['history', unreadHeadHistory]]), identity: datedIdentity }),
    { holder: { tag: 'AAA', id: 2, slot: 7 }, extent: [100, 450], gaps: [], surface: '#161616', yMin: null, yMax: null },
  );
  assert.equal(unreadHeadModel.empty?.kind, 'unavailable');
  assert.equal(unreadHeadModel.empty?.message, 'Occupation settings were not recorded at these hours');
  assert.deepEqual(unreadHeadModel.auditRows, []);

  const lane = createLaneView(timeChartLane, () => {});
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
