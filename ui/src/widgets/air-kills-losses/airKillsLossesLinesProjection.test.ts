import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardSource } from '../../dashboard/client';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { FollowQuery } from '../../dashboard/regionTime.ts';
import type { TiledLine } from '../../dashboard/tiles/view.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { compileFollowPlan, hoverAsks, readPlanExact } from '../../families/broker.ts';
import type { ArtHandle, ArtProvider, FamilyPlanContext, IdentityProvider } from '../../families/contract.ts';
import { buildTooltipContent } from '../../families/host/tooltips.ts';
import { SceneTiles, serializeTiledBySource } from '../../families/sceneTiles.ts';
import { unitEntries } from '../../families/readUnits.ts';
import type { FrozenGate } from '../../families/sources.ts';
import { deriveTimeChartTooltip, sampleSeriesValueAt } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import { buildLineSubpaths, collectVisibleChartArt } from '../../families/time-chart/painter.ts';
import { DEFAULT_LINE_SETTINGS } from '../../families/time-chart/settings.ts';
import type { MilitaryAirAtReply } from '../../generated/dashboard';
import {
  airKillsLossesLinesDeclaration,
  airKillsLossesLinesProjection,
  airKillsLossesProvenInterval,
  type AirKillsLossesLinesOptions,
} from './airKillsLossesLinesProjection.ts';

const SOURCE = {
  id: 'src',
  revision: 'rev1',
  name: 'Test',
  firstHour: 0,
  lastHour: 500,
  branches: [{ id: '1', firstHour: 0, lastHour: 500 }],
} as unknown as DashboardSource;

const GATE: FrozenGate = {
  recorded: (hour: number | null | undefined) => (typeof hour === 'number' ? hour : null) as never,
  span: (firstHour: number, lastHour: number) => (firstHour <= lastHour ? { firstHour, lastHour } : null) as never,
};

function makeCtx(overrides: Partial<FamilyPlanContext> = {}): FamilyPlanContext {
  return {
    track: { id: 't1', source: SOURCE, branch: { id: '1', firstHour: 0, lastHour: 500, gaps: [] }, axisOffset: 0 } as never,
    branch: { id: '1', firstHour: 0, lastHour: 500, gaps: [] } as never,
    lastRecordedHour: 500,
    widget: {
      id: 'w1',
      kind: 'air-kills-losses',
      trackIds: ['t1'],
      countries: { kind: 'group', group: { type: 'top', count: 5 }, membership: 'hour', aggregate: false },
      options: { view: 'lines', measure: 'taken' },
      time: { mode: 'follow' },
    } as never,
    options: { view: 'lines', measure: 'taken', regions: '', causes: '', endLabels: true },
    following: true,
    aux: undefined,
    countryTag: null,
    countrySubject: null,
    countryName: '',
    countries: {
      all: false,
      tags: [],
      holders: null,
      every: [countryKey(1), countryKey(2), countryKey(3)],
      top: 5,
      membership: null,
      membershipRuns: null,
      unobserved: [],
      filter: { passing: null, unrecorded: [], bounds: [] },
      factions: null,
      factionUnobserved: [],
      factionIncomplete: [],
      rankedAt: () => [],
      reason: null,
      known: () => true,
      unobservedIn: () => false,
    },
    gateSpan: (first, last) => ({ firstHour: first, lastHour: last }) as never,
    staticSpan: null,
    ...overrides,
  };
}

function makeOptions(overrides: Partial<AirKillsLossesLinesOptions> = {}): AirKillsLossesLinesOptions {
  return {
    tags: [],
    top: 5,
    rankMetric: 'taken',
    holders: null,
    membership: null,
    endLabels: true,
    settings: DEFAULT_LINE_SETTINGS,
    extent: [0, 500],
    gaps: [],
    surface: '#11161d',
    ...overrides,
  };
}

function airReply(overrides: Partial<MilitaryAirAtReply> = {}): MilitaryAirAtReply {
  return {
    family: 'airLosses',
    hour: 100,
    countedFrom: 0,
    validUntil: 199,
    missing: null,
    rankMissing: null,
    rows: [
      {
        tag: 'GER',
        identity: '1:10',
        segmentFirstHour: 0,
        dealt: '45000000',
        taken: '20000000',
        complete: true,
        partial: null,
        ratioHundredths: '225',
        causes: [{ cause: 1, name: 'Aircraft combat', missing: null, taken: '15000000' }],
      },
      {
        tag: 'SOV',
        identity: '2:20',
        segmentFirstHour: 0,
        dealt: '12000000',
        taken: '10000000',
        complete: true,
        partial: null,
        ratioHundredths: '120',
        causes: [],
      },
    ],
    absent: [],
    unattributed: ['0', '0'],
    warnings: [],
    ...overrides,
  };
}

function fakeIdentity(names: Readonly<Record<string, { readonly name: string; readonly color: string }>>): IdentityProvider {
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

test('air kills & losses lines projects slot-keyed dual lines, whole-number axis, and end labels on the ranked side', () => {
  const k1 = countryKey(1);
  const k2 = countryKey(2);
  const identity = fakeIdentity({
    [k1]: { name: 'German Reich', color: '#6d7874' },
    [k2]: { name: 'Soviet Union', color: '#993333' },
  });
  const readView = createBoundedReadView({
    hour: displayedHour(100),
    panels: new Map([['head', airReply()]]),
    identity,
  });

  const model = airKillsLossesLinesProjection.project(readView, makeOptions({ rankMetric: 'taken' }));
  const plan = airKillsLossesLinesDeclaration.sources(makeCtx(), { following: true, staticSpan: null });
  const asks = hoverAsks(plan, 90, name => model.drawnKeys?.[name] ?? [countryKey(3)]);
  assert.deepEqual(model.drawnKeys, { dealt: [k1, k2], taken: [k1, k2] });
  assert.deepEqual(asks[0]?.request.params.countries, [1, 2]);
  assert.equal(model.yAxis.decimals, 0);
  assert.equal(model.series.length, 4);
  assert.equal(model.series[0]!.id, `${k1}\u0000dealt`);
  assert.equal(model.series[0]!.headValue, 450);
  assert.equal(model.series[0]!.endLabel, null);
  assert.equal(model.series[1]!.id, `${k1}\u0000taken`);
  assert.equal(model.series[1]!.headValue, 200);
  assert.equal(model.series[1]!.endLabel?.name, 'German Reich');
  assert.equal(model.series[1]!.endLabel?.value, '450 / 200', 'the one end label carries enemy losses and own losses');
  assert.equal(model.series[3]!.endLabel?.name, 'Soviet Union');
  assert.equal(model.series[3]!.endLabel?.value, '120 / 100');
  assert.deepEqual(model.legend?.map(l => l.label), ['Solid: enemy losses', 'Dashed: own losses']);
});

test('a partial total is labelled partial and audited as a recorded subtotal with its reason, never as exact or as a bound', () => {
  const k1 = countryKey(1);
  const reason = "Some events since the totals' start have no recorded country";
  const rows = airReply().rows.map(row => ({ ...row, complete: false, partial: reason, ratioHundredths: null }));
  const readView = createBoundedReadView({
    hour: displayedHour(100),
    panels: new Map([['head', airReply({ rows, unattributed: ['0', '7'] })]]),
    identity: fakeIdentity({ [k1]: { name: 'German Reich', color: '#6d7874' } }),
  });
  const model = airKillsLossesLinesProjection.project(readView, makeOptions({ rankMetric: 'taken' }));
  assert.equal(model.series[1]!.endLabel?.value, '450 / 200 (partial)', 'a partial total is named as one, never as a bound: losses can be corrected downwards');
  const audited = model.auditRows?.find(row => row.key === `${k1}\u0000taken`);
  assert.equal(audited?.value, `200 (recorded subtotal: ${reason})`, `the audit gives the recorded figure and why it is a subtotal: ${JSON.stringify(audited)}`);
});

test('unrecorded air losses stay not recorded with their reason, never zero or blank', () => {
  const readView = createBoundedReadView({
    hour: displayedHour(100),
    panels: new Map([['head', airReply({ missing: 'Aircraft loss events were not recorded at this hour', rows: [] })]]),
  });
  const model = airKillsLossesLinesProjection.project(readView, makeOptions());
  assert.equal(model.series.length, 0);
  assert.equal(model.empty?.kind, 'unavailable');
  assert.equal(model.empty?.message, 'Aircraft loss events were not recorded at this hour');
  assert.deepEqual(model.drawnKeys, { dealt: [], taken: [] });
  assert.deepEqual(model.auditRows?.map(r => r.value), ['Aircraft loss events were not recorded at this hour']);
});

test('a recorded hour without ranked losses publishes a settled empty roster', () => {
  const recorded = createBoundedReadView({ hour: displayedHour(100), panels: new Map([['head', airReply({ rows: [] })]]) });
  const pending = createBoundedReadView({ hour: displayedHour(100) });
  const options = makeOptions();
  assert.deepEqual(airKillsLossesLinesProjection.project(recorded, options).followedTileRanks, { head: { tags: [], refused: null } });
  assert.equal(airKillsLossesLinesProjection.project(pending, options).followedTileRanks, undefined);
});

test('hover detail scopes requests and rows to dated eligible slots and preserves absent countries as missing', () => {
  const k1 = countryKey(1);
  const k2 = countryKey(2);
  const ctx = makeCtx({
    widget: {
      id: 'w1',
      kind: 'air-kills-losses',
      trackIds: ['t1'],
      countries: { kind: 'tags', tags: [k1, k2] },
      options: { view: 'lines', measure: 'taken' },
      time: { mode: 'follow' },
    } as never,
    countries: {
      ...makeCtx().countries,
      top: null,
      tags: [k1, k2],
      membership: { [k1]: [[0, 500]], [k2]: [[120, 500]] },
    },
  });
  const plan = airKillsLossesLinesDeclaration.sources(ctx, { following: true, staticSpan: null });
  const asksBefore = hoverAsks(plan, 110);
  assert.equal(asksBefore.length, 1);
  assert.deepEqual((asksBefore[0]!.request.params as { countries: unknown }).countries, [1]);

  const identity = fakeIdentity({
    [k1]: { name: 'German Reich', color: '#6d7874' },
    [k2]: { name: 'Soviet Union', color: '#993333' },
  });
  const readView = createBoundedReadView({
    hour: displayedHour(200),
    panels: new Map([['head', airReply({ hour: 200 })]]),
    hovers: new Map([['hover', { hour: 110, data: airReply({ hour: 110, rows: [] }) }]]),
    identity,
  });
  const model = airKillsLossesLinesProjection.project(readView, makeOptions({
    top: null,
    tags: [k1, k2],
    membership: { [k1]: [[0, 500]], [k2]: [[120, 500]] },
  }));
  assert.ok(model.hoverDetail);
  assert.equal(model.hoverDetail.items.length, 1);
  assert.equal(model.hoverDetail.items[0]!.label, 'German Reich');
  assert.equal(model.hoverDetail.items[0]!.missingReason, 'This country is not present at this date.');

  const precise = airKillsLossesLinesProjection.project(createBoundedReadView({
    hour: displayedHour(200),
    panels: new Map([['head', airReply({ hour: 200 })]]),
    hovers: new Map([['hover', { hour: 150, data: airReply({ hour: 150, rows: [{ ...airReply().rows[0]!, taken: '123456' }] }) }]]),
    identity,
  }), makeOptions({ top: null, tags: [k1], membership: null }));
  assert.equal(precise.hoverDetail?.items.find(item => item.label === 'Own losses')?.value, '1.23456', 'inspection carries the recorded five decimals');
});

test('a country chosen by its tag is one line, joined to its head row by tag or slot, never two', () => {
  const model = airKillsLossesLinesProjection.project(createBoundedReadView({ hour: displayedHour(200), panels: new Map([['head', airReply({ hour: 200 })]]) }),
    makeOptions({ top: null, tags: ['GER'], membership: null }));
  assert.deepEqual(model.series.map(s => s.id.replace(/\u0000/g, '|')), ['GER|dealt', 'GER|taken']);
  assert.equal(model.series[0]!.headValue, 450);
  assert.equal(model.summaryText, 'Loss totals include corrections.', 'a description is plain text beside the title');
  assert.equal(model.note, undefined, 'the partial-data note is kept for losses no country is recorded for');
});

test('the exact audit reads the dated head, so a correct picture has an exact read to agree with', async () => {
  const asked: string[] = [];
  const client = { product: async (request: { product: string }) => { asked.push(request.product); return { data: airReply() }; } } as never;
  for (const countries of [makeCtx().countries, { ...makeCtx().countries, top: null, tags: [countryKey(1), countryKey(2)] }]) {
    const ctx = makeCtx({ countries });
    asked.length = 0;
    const exact = await readPlanExact(airKillsLossesLinesDeclaration.sources(ctx, { following: false, staticSpan: null }), 100,
      { client, following: false, timeMode: 'pin', branch: { firstHour: 0, lastHour: 500 }, window: null, staticSpan: null, gate: GATE });
    assert.ok(exact, 'an exact read exists');
    assert.deepEqual(asked, ['military-air-at']);
    const model = airKillsLossesLinesProjection.project(exact, makeOptions({ top: countries.top, tags: countries.tags }));
    assert.deepEqual(model.auditRows?.map(row => row.value), ['450', '200', '120', '100'], 'both countries, both measures, from the head');
  }
});

test('the military air head follows changes without an exact playback request', () => {
  const ctx = makeCtx();
  const plan = airKillsLossesLinesDeclaration.sources(ctx, { following: true, staticSpan: null });
  const { subscriptions } = compileFollowPlan(plan, query => ({ query: query as FollowQuery | null, refused: null }));
  assert.deepEqual(subscriptions.map(sub => [sub.name, sub.channel, sub.product]), [['head', 'panel', 'military-air-at']]);
  assert.ok(unitEntries(plan, true, subscriptions).every(entry => entry.name !== 'head'));
  assert.deepEqual(airKillsLossesProvenInterval(airReply({ hour: 100, validUntil: 250 }), 100), [100, 250]);
});

test('partial tile spans keep drawing the witnessed subtotal and report partial state on hover, and air end labels demand and collect flag art', () => {
  const k1 = countryKey(1);
  const reason = "Some events since the totals' start have no recorded country";
  const line: TiledLine = {
    tag: k1,
    head: 'test',
    decimals: 5,
    summary: {
      from: 0,
      to: 200,
      width: 200,
      buckets: [],
      runs: [
        { firstHour: 0, lastHour: 49, value: 100 },
        { firstHour: 50, lastHour: 100, value: 200 },
      ],
    },
    missing: [],
    partial: [{ firstHour: 50, lastHour: 100, reason }],
    parts: [],
    marks: null,
  };
  const serialized = structuredClone(serializeTiledBySource({
    taken: new Map([[k1, line]]),
    dealt: new Map([[k1, line]]),
  }));
  const hydratedSeries = new SceneTiles().series(
    { hour: 100, seek: 0, speed: 0, answers: new Map() } as never,
    { sources: {} },
    { tiledBySource: serialized },
    100,
  )!;
  const demandedFlags: ArtHandle[] = [];
  const flagUrl = 'asset://flags/ger-panel.png';
  const identity: IdentityProvider = {
    resolve(kind, key, atHour) {
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: 'GER' },
        name: { status: 'ready', value: 'German Reich' },
        colour: { status: 'ready', value: '#6d7874' },
        flagChoice: { status: 'ready', value: '[["GER",null,null],[null,null,null]]' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
  const art: ArtProvider = {
    resolve(handle) {
      if (handle.kind === 'flag') demandedFlags.push(handle);
      return {
        status: 'ready',
        url: flagUrl,
        assetId: 'ger',
        file: 'ger.tga',
        origin: { kind: 'baseGame' },
        provenance: { fromCurrentInstall: true, label: 'Current installation', scopeKey: 'current' },
        frame: { frameIndex: 0, frameCount: 1, frameWidth: 18, frameHeight: 12, textureWidth: 18, textureHeight: 12, outputWidth: 18, outputHeight: 12 },
      };
    },
  };
  const readView = createBoundedReadView({
    hour: displayedHour(100),
    panels: new Map([['head', airReply({
      rows: [{ ...airReply().rows[0]!, complete: false, partial: reason, ratioHundredths: null }],
    })]]),
    series: hydratedSeries,
    identity,
    art,
  });
  const model = airKillsLossesLinesProjection.project(readView, makeOptions({ rankMetric: 'taken' }));
  const taken = model.series.find(s => s.id === `${k1}\u0000taken`)!;
  assert.ok(taken.partialSpans?.length === 1);
  assert.equal(sampleSeriesValueAt(taken, 75, 100)?.value, 200, 'partial span still samples the witnessed subtotal');
  const layout = layoutTimeChart(model, 720, 260, 'Segoe UI', text => text.length * 6);
  const { subpaths } = buildLineSubpaths(taken, layout, 100);
  assert.ok(subpaths.flat().length >= 2, 'line still draws through partial spans');
  const tip = deriveTimeChartTooltip(model, layout, 'plot:75');
  assert.ok(tip?.payload);
  const content = buildTooltipContent(tip!.payload!);
  const row = content.rows.find(r => r.label === 'German Reich · Own losses');
  assert.deepEqual(row?.valueState, { kind: 'partial', text: '200', reason }, 'hover inside partial span reports witnessed subtotal with partial reason');
  assert.ok(demandedFlags.some(h => h.kind === 'flag' && h.size === 'panel'), 'air series demands panel flag handles');
  assert.ok(collectVisibleChartArt(model, layout, null).includes(flagUrl), 'visible chart art collects air end-label flag URLs');

  const exactView = createBoundedReadView({
    hour: displayedHour(100),
    panels: new Map([['head', airReply()]]),
    hovers: new Map([['hover', {
      hour: 75,
      data: airReply({
        hour: 75,
        rows: [{ ...airReply().rows[0]!, complete: false, partial: reason, ratioHundredths: null }],
      }),
    }]]),
    identity,
    art,
  });
  const exactModel = airKillsLossesLinesProjection.project(exactView, makeOptions({ rankMetric: 'taken' }));
  const exactTip = deriveTimeChartTooltip(exactModel, layoutTimeChart(exactModel, 720, 260, 'Segoe UI', text => text.length * 6), 'plot:75');
  const exactContent = buildTooltipContent(exactTip!.payload!);
  const exactRow = exactContent.rows.find(r => r.label === 'Own losses');
  assert.deepEqual(exactRow?.valueState, { kind: 'partial', text: '200', reason }, 'exact hover preserves partial state and reason');
});
