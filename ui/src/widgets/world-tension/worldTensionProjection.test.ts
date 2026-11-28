import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { readyArt } from '../../families/art/types.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import type { DashboardClient } from '../../dashboard/client';
import { hoverAsks, readPlanExact } from '../../families/broker.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import type { FrozenGate } from '../../families/sources.ts';
import type { TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import type { TimeChartModel } from '../../families/time-chart/model.ts';
import { IdentityIndex } from '../../families/identity/index.ts';
import { createSnapshotIdentityProvider } from '../../families/identity/provider.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import { buildLineSubpaths, drawTimeChart } from '../../families/time-chart/painter.ts';
import type { Canvas2D, MeasureText } from '../../charts/textWidth.ts';
import type { WorldTensionReply, WorldTensionStackReply } from '../../generated/dashboard';
import { FAMILY_VIEWS } from '../familyManifest.ts';
import { worldTensionEventsDeclaration, worldTensionEventsProjection } from './worldTensionEventsProjection.ts';
import { worldTensionLinesDeclaration, worldTensionLinesProjection } from './worldTensionLinesProjection.ts';
import { worldTensionDeclaration, worldTensionProjection } from './worldTensionProjection.ts';
import { DEFAULT_FAMILY_THEME } from '../../families/theme.fixture.ts';
import { tensionStackAt } from './stackHistory.ts';
import { deriveRowTooltip, resolveTableRows } from '../../families/table/model.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import { EVENTS_TABLE_BASE } from './definition.ts';

const H1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
const H1941 = engineHour({ year: 1941, month: 9, day: 15, hour: 1 })!;
const measure: MeasureText = text => text.length * 6.5;

function makeIdentity() {
  const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: 'b', content: 0 }, [H1936, H1941 + 1000]);
  index.setFacts({
    branchId: 'b',
    known: [[H1936, H1941 + 1000]],
    names: ['German Reich', 'Japan', 'Italy'],
    reasons: [],
    flags: [
      { keys: ['GER_fascism', null, null], reasons: [null, null, null] },
      { keys: ['JAP_fascism', null, null], reasons: [null, null, null] },
      { keys: ['ITA_fascism', null, null], reasons: [null, null, null] },
    ],
    countries: [
      { tag: 'GER', runs: [[H1936, 0, 0x454545, -1, 0]] },
      { tag: 'JAP', runs: [[H1936, 1, 0xcc4444, -1, 1]] },
      { tag: 'ITA', runs: [[H1936, 2, 0x44aa55, -1, 2]] },
    ],
    warnings: [],
  });
  return createSnapshotIdentityProvider(index);
}

function held(tag: string, remembered = false) {
  return [{ tag, firstHour: H1936, lastHour: H1941, remembered }];
}

function sampleStack(): WorldTensionStackReply {
  return {
    branchId: '1',
    tensionScale: 1000,
    contributionScale: 100000,
    ceilingRaw: '10000000',
    endHour: H1941,
    cappedEndTension: 100,
    cappedEndTensionRaw: '100000',
    endMissing: null,
    endSumRaw: '10000000',
    downsampled: false,
    refused: null,
    hours: [H1936, H1936 + 5000, H1936 + 20000],
    total: [0, 35, 100],
    totalRaw: ['0', '35000', '100000'],
    sumRaw: ['0', '3500000', '10000000'],
    bands: [
      { slot: 1, tag: 'GER', remembered: false, holders: held('GER'), values: [0, 20, 60], valuesRaw: ['0', '2000000', '6000000'], minValues: [], maxValues: [], endValue: 60, endRaw: '6000000', endMissing: null },
      { slot: 2, tag: 'JAP', remembered: false, holders: held('JAP'), values: [0, 10, 25], valuesRaw: ['0', '1000000', '2500000'], minValues: [], maxValues: [], endValue: 25, endRaw: '2500000', endMissing: null },
      { slot: 3, tag: 'ITA', remembered: false, holders: held('ITA'), values: [0, 2, 3], valuesRaw: ['0', '200000', '300000'], minValues: [], maxValues: [], endValue: 3, endRaw: '300000', endMissing: null },
    ],
    other: { count: 31, values: [0, 3, 12], valuesRaw: ['0', '300000', '1200000'], endValue: 12, endRaw: '1200000', endMissing: null },
    events: [
      { hour: H1936 + 5000, country: 9001, slot: 1, secondCountry: 0, place: 0, tag: 'GER', remembered: false, reason: 'Remilitarised the Rhineland', addedRaw: '500000', firstRead: H1936 + 5000, lastRead: null },
      { hour: H1941 + 200, country: 9002, slot: 2, secondCountry: 0, place: 0, tag: 'JAP', remembered: false, reason: 'Future event', addedRaw: '1000000', firstRead: H1941 + 200, lastRead: null },
    ],
    missing: [],
    nextChangeHour: H1941 + 24,
    warnings: [],
  };
}

describe('world-tension projections', () => {
  it('R3-S4: tension event inspection retains the recorded ratio behind the display cut', () => {
    const reply = sampleStack();
    reply.contributionScale = 3;
    reply.events = [{ ...reply.events[0]!, reason: 'E01', addedRaw: '1' }];
    const model = worldTensionEventsProjection.project(createBoundedReadView({ hour: H1941, panels: new Map([['main', reply]]) }), { branchFirst: H1936 });
    assert.deepEqual(model.rows[0]!.cells.added!.value, { kind: 'known', value: '+0.3%' });
    const flat = resolveTableRows(model, tableSettingsOf(undefined, model.columns, EVENTS_TABLE_BASE));
    const found = flat.find(row => row.kind === 'data')!;
    assert.ok(found.kind === 'data');
    const inspected = deriveRowTooltip(found, model.columns).rows.find(row => row.label === 'Adds')!;
    assert.deepEqual(inspected.rawValue, { kind: 'known', value: '+1/3%' });
    assert.equal(inspected.valueState!.text, '+1/3%');
  });
  it('combines the closed samples with the dated edge in rank and event order after a seek', () => {
    const full = sampleStack();
    const pieces = ([true, false] as const).map(closed => {
      const take = <T>(values: T[]) => closed ? values.slice(0, 1) : values.slice(1);
      return {
        ...full, pieceStart: H1941, events: [full.events[closed ? 1 : 0]!],
        hours: take(full.hours), total: take(full.total), totalRaw: take(full.totalRaw), sumRaw: take(full.sumRaw),
        bands: full.bands.map(band => ({ ...band, holders: closed ? [] : band.holders,
          values: take(band.values), valuesRaw: take(band.valuesRaw) })),
        other: { ...full.other!, values: take(full.other!.values), valuesRaw: take(full.other!.valuesRaw) },
      };
    });
    const history = { ...pieces[0]!, bands: [...pieces[0]!.bands].reverse() };
    const edge = { ...pieces[1]!, eventPositions: [0] };
    assert.deepEqual(tensionStackAt(history, edge), full);
    assert.equal(tensionStackAt({ ...history, branchId: '2' }, edge), null);
    assert.equal(tensionStackAt({ ...history, pieceStart: H1936 }, edge), null);

    const options = { extent: [H1936, H1941] as const, surface: '#111111', gaps: [] };
    const at = (panels: [string, WorldTensionStackReply][]) => worldTensionLinesProjection.project(
      createBoundedReadView({ hour: H1941, panels: new Map(panels), identity: makeIdentity() }), options,
    );
    assert.deepEqual(at([['history', history], ['main', edge]]), at([['main', full]]));
    assert.deepEqual(at([['history', { ...history, pieceStart: H1936 }], ['main', edge]]).series, []);
  });
  it('hovers only installed contributors and Other in Share mode and withholds pre-membership readings', () => {
    const at = H1936 + 5;
    const base = sampleStack();
    const main = { ...base, bands: [base.bands[0]!], other: { ...base.other!, count: 1 } };
    const data = {
      tensionScale: 1000, contributionScale: 100000, contributions: [
        { slot: 1, tag: 'GER', remembered: false, reading: { Ok: '8000000' } },
      ], sources: [], warnings: [], missing: null,
      live: {
        rows: ['world'], kinds: ['tension', 'sum'].map(id => ({ id, op: 'value', scale: 1 })),
        groups: [
          { table: 1, columns: [0], keys: ['0'], values: [50000] },
          { table: 2, columns: [0], keys: ['0'], values: [20000000] },
        ],
        cells: [0, 0, 1, 0, 0, 0, 0, 1, 1, 1, 0, 0],
      },
    };
    const view = (detail: unknown, picture: WorldTensionStackReply) => createBoundedReadView({
      hour: H1941, panels: new Map([['main', picture]]), identity: makeIdentity(),
      hovers: new Map([['hover', { hour: at, data: detail }]]),
    });
    const opts = { extent: [H1936, H1941] as const, surface: '#111111', gaps: [] };
    const stack = worldTensionProjection.project(view(data, { ...main, bands: [{ ...main.bands[0]!, remembered: true }] }),
      { ...opts, layers: 1, values: 'share', showMarkers: false });
    const items = stack.hoverDetail!.items.filter(item => !['World tension', 'Uncapped sum'].includes(item.label));
    assert.deepEqual(items.map(item => [item.label, item.value]), [['German Reich', '+20%'], ['Other (1)', '+30%']]);
    const ctx = {
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1', firstHour: H1936, lastHour: H1941, gaps: [] },
      countries: { all: false, tags: ['GER'], top: null }, options: { values: 'share' },
      widget: { countries: { kind: 'tags', tags: ['GER'] } },
    } as unknown as FamilyPlanContext;
    const bounds = { width: 800, height: 300, following: false, staticSpan: null };
    const plan = worldTensionDeclaration.sources(ctx, bounds);
    const followedLines = worldTensionLinesDeclaration.sources(ctx, { ...bounds, following: true });
    assert.equal(followedLines.entries.filter(entry => entry.read.shape === 'branch-to-hour').length, 0);
    assert.deepEqual(followedLines.entries.flatMap(({ name, read }) => read.shape === 'point' && read.channel === 'panel'
      ? [[name, read.followQuery?.piece, read.hourParam]] : []), [
      ['history', 'history', 'lastHour'], ['main', 'edge', 'lastHour'],
    ]);
    const pausedLines = worldTensionLinesDeclaration.sources(ctx, bounds);
    assert.equal(pausedLines.entries.filter(entry => entry.read.shape === 'branch-to-hour').length, 1);
    assert.deepEqual(hoverAsks(plan, at, name => stack.drawnKeys?.[name])[0]?.request.params,
      { branchId: '1', values: 'share', group: null, countries: [1], hour: at });
    assert.deepEqual(hoverAsks(plan, at, () => [])[0]?.request.params,
      { branchId: '1', values: 'share', group: null, countries: [], hour: at });
    const reason = 'Not a member of the selected group at this hour.';
    const lines = worldTensionLinesProjection.project(view({ ...data, contributions: [
      { slot: 1, tag: 'GER', remembered: false, reading: { Err: reason } },
    ] }, { ...main, other: null }), opts);
    const group = { type: 'humans', membership: 'hour' };
    const groupPlan = worldTensionLinesDeclaration.sources({ ...ctx,
      widget: { ...ctx.widget, countries: { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } },
    }, bounds);
    assert.deepEqual(hoverAsks(groupPlan, at, name => lines.drawnKeys?.[name])[0]?.request.params,
      { branchId: '1', values: 'recorded', group, countries: [1], hour: at });
    assert.equal(lines.series[0]?.exact?.value, null);
    assert.equal(lines.series[0]?.exact?.reason, reason);
  });

  it('keeps one series per slot and shows the holder name, flag and colour at the inspected hour', () => {
    const change = H1936 + 10000;
    const sample = sampleStack();
    const previous = { tag: 'JAP', firstHour: H1936 + 2, lastHour: change - 1, remembered: false };
    const current = { tag: 'GER', firstHour: change, lastHour: change + 4, remembered: false };
    const reply = { ...sample, bands: [{ ...sample.bands[0]!, holders: [previous, current, { ...current, firstHour: change + 5, lastHour: H1941 - 1, remembered: true }] }], other: null, events: [] };
    const view = createBoundedReadView({ hour: H1941, panels: new Map([['main', reply]]), identity: makeIdentity(),
      art: { resolve: handle => readyArt(handle.kind === 'flag' ? handle.choice : 'world') } });
    const opts = { extent: [H1936, H1941] as const, surface: '#111111', gaps: [] };
    const lines = worldTensionLinesProjection.project(view, opts);
    const stack = worldTensionProjection.project(view, { ...opts, values: 'recorded', layers: 1, showMarkers: false });
    assert.deepEqual(lines.series.map(line => line.id), ['slot:1']);
    assert.deepEqual(stack.stack?.bands.map(band => band.id), ['slot:1']);
    for (const model of [lines, stack]) {
      const layout = layoutTimeChart(model, 720, 260, 'Segoe UI', measure);
      const at = (hour: number) => deriveTimeChartTooltip(model, layout, `plot:${hour}`)?.payload?.items?.find(item => item.label !== 'World tension');
      const early = at(change - 1);
      const late = at(change);
      assert.equal(early?.label, 'Japan');
      assert.equal(late?.label, 'German Reich');
      assert.match(early?.countryFlag?.flag ?? '', /JAP/);
      assert.match(late?.countryFlag?.flag ?? '', /GER/);
      assert.notEqual(early?.swatch, late?.swatch);
      const unknown = at(H1936 + 1);
      assert.equal(unknown?.label, 'Unknown country');
      assert.equal(unknown?.countryFlag, undefined);
      assert.equal(at(change + 5)?.label, 'German Reich (former)');
      assert.equal(at(H1941)?.label, 'German Reich (former)');
      assert.equal(at(H1941)?.countryFlag?.flag, late?.countryFlag?.flag);
    }
  });

  it('projects stepped stack with Other at bottom, leader on top, shortName at small, and clamps future events', () => {
    const reply = sampleStack();
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      identity: makeIdentity(),
    });
    const model = worldTensionProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: true,
      gaps: [],
    });
    assert.equal(model.mode, 'stack');
    assert.equal(model.stack?.step, true);
    assert.equal(model.stack?.bands[0]?.key, 'slot:1');
    assert.equal(model.stack?.bands.at(-1)?.id, 'other');
    assert.equal(model.stack?.bands.at(-1)?.endLabel?.name, 'Other (31)');
    assert.equal(model.stack?.bands.at(-1)?.endLabel?.shortName, 'Other');
    assert.equal(model.stack?.total?.endLabel?.value, '100%');
    assert.equal(model.markers?.length, 1, 'future event after displayed hour is excluded');

    const fullLayout = layoutTimeChart(model, 960, 280, 'Segoe UI', measure);
    assert.ok(fullLayout.endLabels.some(l => l.name === 'Other (31)'));
    const totalLabel = fullLayout.endLabels.find(l => l.seriesId === '__total__')!;
    const labelHit = hitTestTimeChart(model, fullLayout, [totalLabel.x + 10, totalLabel.y]);
    assert.equal(labelHit, 'label:__total__');
    const tip = deriveTimeChartTooltip(model, fullLayout, labelHit);
    assert.equal(tip?.payload?.title, 'World tension');

    const minLayout = layoutTimeChart(model, 476, 186, 'Segoe UI', measure);
    assert.ok(minLayout.endLabels.some(l => l.name === 'Other'), 'min layout uses shortName Other');
    assert.equal(minLayout.endLabels.some(l => l.seriesId === 'slot:3'), false, 'thin <10px band label hidden at min size');

    const ops: string[] = [];
    const ctx = new Proxy({}, {
      get: (_t, prop) => {
        if (prop === 'measureText') return (t: string) => ({ width: t.length * 6.5 });
        return (...args: unknown[]) => { ops.push(String(prop)); return args; };
      },
    }) as unknown as Canvas2D;
    drawTimeChart(ctx, model, fullLayout, DEFAULT_FAMILY_THEME, `plot:${H1936 + 5000}`, 1);
    assert.ok(ops.includes('fill') && ops.includes('arc'), 'stepped stack paints bands and hover dot');
  });

  it('projects world-tension events table view with dated events clipped to displayed hour', () => {
    const reply = sampleStack();
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      identity: makeIdentity(),
    });
    const table = worldTensionEventsProjection.project(readView, { branchFirst: H1936 });
    assert.equal(table.rows.length, 1);
    assert.equal(table.rows[0]?.name, 'Remilitarised the Rhineland');

    const thirds = { ...reply, contributionScale: 3, events: [{ ...reply.events[0]!, addedRaw: '1' }] };
    const thirdsTable = worldTensionEventsProjection.project(
      createBoundedReadView({ hour: H1941, panels: new Map([['main', thirds]]), identity: makeIdentity() }),
      { branchFirst: H1936 },
    );
    assert.deepEqual(thirdsTable.rows[0]?.cells.added?.value, { kind: 'known', value: '+0.3%' }, 'R2-K5: a non-decimal scale is cut from the recorded integer and its denominator');
  });

  it('projects world-tension lines view with top-8 coloured lines and stepped inline points', () => {
    const reply = sampleStack();
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      identity: makeIdentity(),
    });
    const model = worldTensionLinesProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    assert.equal(model.mode, 'lines');
    assert.equal(model.series.length, 3);
    assert.equal(model.series[0]?.key, 'slot:1');
    assert.equal(model.series[0]?.step, true);
    assert.equal(model.series[0]?.endLabel?.value, '+60%');
    assert.equal(model.series[0]?.points?.length, 3);
  });

  it('K1: world-tension preserves small shares as nonzero and never turns non-finite inputs into 0%', () => {
    const reply = sampleStack();
    const withEdge = {
      ...reply,
      bands: [
        { slot: 1, tag: 'GER', remembered: false, holders: held('GER'), values: [0, 0.4, 0.4], valuesRaw: ['0', '40000', '40000'], minValues: [], maxValues: [], endValue: 0.4, endRaw: '40000', endMissing: null },
        { slot: 2, tag: 'JAP', remembered: false, holders: held('JAP'), values: [0, 0.04, 0.04], valuesRaw: ['0', '4000', '4000'], minValues: [], maxValues: [], endValue: 0.04, endRaw: '4000', endMissing: null },
        { slot: 3, tag: 'ITA', remembered: false, holders: held('ITA'), values: [0, 1, 1], valuesRaw: ['0', '100000', '100000'], minValues: [], maxValues: [], endValue: Number.NaN, endRaw: '0', endMissing: null },
      ],
      other: null,
    };
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', withEdge]]),
      identity: makeIdentity(),
    });
    const model = worldTensionProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: false,
      gaps: [],
    });
    assert.equal(model.stack?.bands[0]?.endLabel?.value, '+0.4%');
    assert.equal(model.stack?.bands[1]?.endLabel?.value, '+0.04%');
    assert.equal(model.stack?.bands[2]?.endLabel, null);
  });

  it('S2: stacks negative contributions below zero, keeps recorded world total in label and audit, preserves witnessed bands without world scalar, and omits end labels during a gap', () => {
    const negReply: WorldTensionStackReply = {
      branchId: '1',
      tensionScale: 1000,
      contributionScale: 100000,
      ceilingRaw: '10000000',
      endHour: H1936 + 5000,
      cappedEndTension: 6.5,
      cappedEndTensionRaw: '6500',
      endMissing: null,
      endSumRaw: '650000',
      downsampled: false,
      refused: null,
      hours: [H1936, H1936 + 5000],
      total: [0, 6.5],
      totalRaw: ['0', '650000'],
      sumRaw: ['0', '650000'],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, holders: held('GER'), values: [0, 4], valuesRaw: ['0', '400000'], minValues: [], maxValues: [], endValue: 4, endRaw: '400000', endMissing: null },
        { slot: 2, tag: 'JAP', remembered: false, holders: held('JAP'), values: [0, 3], valuesRaw: ['0', '300000'], minValues: [], maxValues: [], endValue: 3, endRaw: '300000', endMissing: null },
        { slot: 3, tag: 'ITA', remembered: false, holders: held('ITA'), values: [0, -0.5], valuesRaw: ['0', '-50000'], minValues: [], maxValues: [], endValue: -0.5, endRaw: '-50000', endMissing: null },
      ],
      other: null,
      events: [],
      missing: [],
      nextChangeHour: null,
      warnings: [],
    };
    const negModel = worldTensionProjection.project(
      createBoundedReadView({ hour: H1936 + 5000, panels: new Map([['main', negReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5000], surface: '#111111', layers: 5, values: 'recorded', showMarkers: false, gaps: [] },
    );
    assert.equal(negModel.yAxis.min, undefined, 'negative contributions must not be clipped at min: 0');
    const negBand = negModel.stack?.bands.find(b => b.id === 'slot:3');
    assert.ok(negBand?.endLabel, 'negative contribution band keeps its end label');
    assert.equal(negModel.stack?.total?.endLabel?.value, '6.5%', 'recorded world total label is 6.5%');
    assert.ok(negModel.auditRows?.some(r => r.row === '__total__' && r.value === '6.5%'));
    assert.ok(negModel.auditRows?.some(r => r.row === 'slot:3' && r.value === '\u22120.5%'), 'negative contribution appears in audit');

    const negLayout = layoutTimeChart(negModel, 720, 260, 'Segoe UI', measure);
    assert.ok(negLayout.yDomain[0] < 0, 'stack yDomain includes room below zero for negative bands');
    const itaLabel = negLayout.endLabels.find(l => l.seriesId === 'slot:3');
    const gerLabel = negLayout.endLabels.find(l => l.seriesId === 'slot:1');
    assert.ok(itaLabel && gerLabel, 'both positive and negative band labels are placed');
    assert.ok(itaLabel!.anchorY > negLayout.toY(0), 'negative band anchors below the zero axis');
    assert.ok(gerLabel!.anchorY < negLayout.toY(0), 'positive band anchors above the zero axis');

    const negLines = worldTensionLinesProjection.project(
      createBoundedReadView({ hour: H1936 + 5000, panels: new Map([['main', negReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5000], surface: '#111111', gaps: [] },
    );
    assert.equal(negLines.yAxis.min, undefined, 'lines view must not clip negative contributions at 0');

    const noWorldScalar: WorldTensionStackReply = {
      ...negReply,
      cappedEndTension: null,
      cappedEndTensionRaw: null,
      endMissing: 'World tension was not recorded at this hour.',
      total: [null, null],
      totalRaw: [null, null],
    };
    const witnessedModel = worldTensionProjection.project(
      createBoundedReadView({ hour: H1936 + 5000, panels: new Map([['main', noWorldScalar]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5000], surface: '#111111', layers: 5, values: 'recorded', showMarkers: false, gaps: [] },
    );
    assert.equal(witnessedModel.empty, undefined, 'missing world scalar must not blank independently witnessed contributions');
    assert.equal(witnessedModel.stack?.bands.length, 3);

    const gapReply: WorldTensionStackReply = {
      ...negReply,
      endHour: null,
      cappedEndTension: null,
      cappedEndTensionRaw: null,
      endMissing: 'Recording gap',
      endSumRaw: null,
      hours: [H1936, H1936 + 5000, H1936 + 5010],
      total: [0, 6.5, null],
      totalRaw: ['0', '650000', null],
      sumRaw: ['0', '650000', null],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, holders: held('GER'), values: [0, 4, null], valuesRaw: ['0', '400000', null], minValues: [], maxValues: [], endValue: null, endRaw: null, endMissing: 'Recording gap' },
        { slot: 2, tag: 'JAP', remembered: false, holders: held('JAP'), values: [0, 3, null], valuesRaw: ['0', '300000', null], minValues: [], maxValues: [], endValue: null, endRaw: null, endMissing: 'Recording gap' },
        { slot: 3, tag: 'ITA', remembered: false, holders: held('ITA'), values: [0, -0.5, null], valuesRaw: ['0', '-50000', null], minValues: [], maxValues: [], endValue: null, endRaw: null, endMissing: 'Recording gap' },
      ],
      missing: [{ firstHour: H1936 + 5010, lastHour: H1936 + 5020, reason: 'Recording gap' }],
    };
    const gapStack = worldTensionProjection.project(
      createBoundedReadView({ hour: H1936 + 5015, panels: new Map([['main', gapReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5015], surface: '#111111', layers: 5, values: 'recorded', showMarkers: false, gaps: [] },
    );
    assert.equal(gapStack.stack?.total?.endLabel, null, 'total end label is absent during a gap');
    assert.deepEqual(gapStack.auditRows?.map(r => [r.row, r.value]), [
      ['__total__', 'Recording gap'],
      ['slot:1', 'Recording gap'],
      ['slot:2', 'Recording gap'],
      ['slot:3', 'Recording gap'],
    ], 'during a gap every drawn end is audited with why it is missing, never a stale value');
    const gapLines = worldTensionLinesProjection.project(
      createBoundedReadView({ hour: H1936 + 5015, panels: new Map([['main', gapReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5015], surface: '#111111', gaps: [] },
    );
    assert.ok(gapLines.series.every(s => s.endLabel === null), 'lines omit end labels during a gap');
  });

  it('R2-K1: hovering a stack prints each figure by its kind: a recorded -2% contribution and an uncapped 125% total, in both value modes', () => {
    const at = H1936 + 5000;
    const reply: WorldTensionStackReply = {
      ...sampleStack(),
      endHour: at,
      cappedEndTension: 100,
      cappedEndTensionRaw: null,
      endMissing: null,
      endSumRaw: null,
      hours: [H1936, at],
      total: [0, 125],
      totalRaw: [null, null],
      sumRaw: [null, null],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, holders: held('GER'), values: [0, 127], valuesRaw: ['0', '12700000'], minValues: [], maxValues: [], endValue: 127, endRaw: '12700000', endMissing: null },
        { slot: 2, tag: 'JAP', remembered: false, holders: held('JAP'), values: [0, -2], valuesRaw: ['0', '-200000'], minValues: [], maxValues: [], endValue: -2, endRaw: '-200000', endMissing: null },
      ],
      other: null,
      events: [],
      nextChangeHour: null,
    };
    for (const values of ['recorded', 'share'] as const) {
      const model = worldTensionProjection.project(
        createBoundedReadView({ hour: at, panels: new Map([['main', reply]]), identity: makeIdentity() }),
        { extent: [H1936, at], surface: '#111111', layers: 5, values, showMarkers: false, gaps: [] },
      );
      const layout = layoutTimeChart(model, 960, 280, 'Segoe UI', measure);
      const ger = layout.endLabels.find(l => l.seriesId === 'slot:1')!;
      const labelTip = deriveTimeChartTooltip(model, layout, hitTestTimeChart(model, layout, [ger.x + 10, ger.y]));
      assert.deepEqual((labelTip?.payload?.items ?? []).map(item => [item.label, item.value]), [['German Reich', '+127%']], values);
      const plotTip = deriveTimeChartTooltip(model, layout, `plot:${at}`);
      assert.deepEqual((plotTip?.payload?.items ?? []).map(item => [item.label, item.value]), [
        ['World tension', '125%'],
        ['German Reich', '+127%'],
        ['Japan', '−2%'],
      ], values);
    }
  });

  it('S10: stack hover reads same-hour events from the hovered-hour world-tension reply, not later playhead revisions', () => {
    const hoverHour = H1936 + 5000;
    const reply: WorldTensionStackReply = {
      ...sampleStack(),
      listingMissing: 'Fixture listing unread',
      events: [
        ...[0, 1].map(place => ({
          hour: hoverHour, country: 9001, slot: 1, secondCountry: 0, place, tag: 'GER', remembered: false, reason: 'Fixture event',
          addedRaw: place === 0 ? '900000' : '100000', markerRaw: place === 0 ? '500000' : '300000',
          firstRead: hoverHour, lastRead: null,
        })),
      ],
    };
    const hoverReply: WorldTensionReply = {
      tensionScale: 1000,
      contributionScale: 100000,
      ceilingRaw: '10000000',
      unnamed: false,
      contributions: [{ slot: 1, tag: 'GER', remembered: false, reading: { Ok: '500000' } }],
      sources: [
        { ordinal: 0, tag: 'GER', remembered: false, reason: 'Fixture event', finalRaw: '500000', dailyRaw: '0', date: hoverHour },
      ],
      missing: null,
      warnings: [],
    };
    const livePayload = {
      live: {
        groups: [
          { table: 1, columns: [0], keys: ['0'], values: [5000] },
          { table: 2, columns: [0], keys: ['0'], values: [500000] },
          { table: 3, columns: [0], keys: ['1'], values: [500000] },
        ],
        rows: ['world', '1'],
        kinds: [
          { id: 'tension', op: 'value', scale: 1 },
          { id: 'sum', op: 'value', scale: 1 },
          { id: 'value', op: 'value', scale: 1 },
        ],
        cells: [
          0, 0, 1, 0, 0, 0,
          0, 1, 1, 1, 0, 0,
          1, 2, 1, 2, 0, 0,
        ],
      },
    };
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      hovers: new Map([['hover', { hour: hoverHour, data: { ...hoverReply, ...livePayload } }]]),
      identity: makeIdentity(),
    });
    const model = worldTensionProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: true,
      gaps: [],
    });
    const evItem = model.hoverDetail?.items.find(item => item.label === 'Fixture event');
    assert.equal(evItem?.value, '+5%', 'hover shows +5% recorded at hoverHour, not +9% revised later at playhead');
    const layout = layoutTimeChart(model, 720, 260, 'Segoe UI', measure);
    const markerIndex = model.markers?.findIndex(marker => marker.hour === hoverHour) ?? -1;
    assert.ok(markerIndex >= 0, 'the event has a marker at its hour');
    const markerTip = deriveTimeChartTooltip(model, layout, `marker:${markerIndex}@${hoverHour}`);
    const listed = (markerTip?.payload?.items ?? []).filter(item => item.label === 'Fixture event');
    assert.deepEqual(listed.map(item => item.value), ['+5%', '+3%']);
    assert.equal(model.markers![markerIndex]!.detail, '+5%');
    const table = worldTensionEventsProjection.project(readView, { branchFirst: H1936 });
    for (const row of table.rows) {
      assert.deepEqual(row.cells.until?.value, { kind: 'notRecorded', reason: 'Fixture listing unread' });
    }
    const empty = worldTensionEventsProjection.project(createBoundedReadView({
      hour: H1941, panels: new Map([['main', { ...reply, events: [] }]]),
    }), { branchFirst: H1936 });
    assert.equal(empty.emptyKind, 'unavailable');
    assert.equal(empty.empty, 'Fixture listing unread');
  });

  it('S11: downsampled world-tension lines paint interval bounds, hover as ranges before exact hover, and show exact hovered-hour contributions once loaded', () => {
    const hoverHour = H1936 + 5000;
    const downsampledReply = {
      ...sampleStack(),
      downsampled: true,
      hours: [H1936, hoverHour - 10, H1941],
      total: [0, 20, 60],
      bands: [
        {
          slot: 1,
          tag: 'GER',
          holders: held('GER'),
          remembered: false,
          values: [0, 20, 60],
          minValues: [0, 10, 60],
          maxValues: [0, 40, 60],
          endValue: 60,
          endRaw: '6000000',
        },
      ],
      other: null,
    } as WorldTensionStackReply;

    const noHoverView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', downsampledReply]]),
      identity: makeIdentity(),
    });
    const lineModel = worldTensionLinesProjection.project(noHoverView, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    const layout = layoutTimeChart(lineModel, 720, 260, 'Segoe UI', measure);
    const tipWithoutExact = deriveTimeChartTooltip(lineModel, layout, `plot:${hoverHour}`);
    const rangeItem = tipWithoutExact?.payload?.items?.find(i => i.label === 'German Reich');
    assert.ok(rangeItem?.value?.includes('\u2013'), `expected interval bounds range on downsampled hover, got ${rangeItem?.value}`);

    const dipping = worldTensionLinesProjection.project(createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', { ...downsampledReply, bands: [{ ...downsampledReply.bands[0]!, minValues: [0, -4, 60] }] }]]),
      identity: makeIdentity(),
    }), { extent: [H1936, H1941], surface: '#111111', gaps: [] });
    assert.notEqual(dipping.yAxis.min, 0, 'a bin that dipped to -4% before ending higher keeps its low on the axis');
    const reducedStack = worldTensionProjection.project(createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', { ...downsampledReply, bands: [{ ...downsampledReply.bands[0]!, valuesRaw: ['0', '2000000', '6000000'] }] }]]),
      identity: makeIdentity(),
    }), { extent: [H1936, H1941], surface: '#111111', layers: 5, values: 'share', showMarkers: false, gaps: [] });
    assert.ok(reducedStack.stack?.bands.every(band => band.exact === undefined), 'a reduced stack claims no exact figure for its bins');
    const dipLayout = layoutTimeChart(dipping, 720, 260, 'Segoe UI', measure);
    assert.ok(dipLayout.yDomain[0] <= -4, `the axis reaches the bin's low: ${dipLayout.yDomain}`);
    const between = deriveTimeChartTooltip(reducedStack, layoutTimeChart(reducedStack, 720, 260, 'Segoe UI', measure), `plot:${hoverHour - 5}`);
    assert.ok(!(between?.payload?.items ?? []).some(item => item.value !== undefined), `inside a grouped span no figure is shown as the hour's: ${JSON.stringify(between?.payload?.items)}`);
    const settled = worldTensionProjection.project(createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', { ...downsampledReply, hours: [H1936, hoverHour - 10], total: [0, 20], bands: [{ ...downsampledReply.bands[0]!, values: [0, 20], minValues: [0, 10], maxValues: [0, 40], valuesRaw: ['0', '2000000'] }] }]]),
      identity: makeIdentity(),
    }), { extent: [H1936, H1941], surface: '#111111', layers: 5, values: 'share', showMarkers: false, gaps: [] });
    const afterLast = deriveTimeChartTooltip(settled, layoutTimeChart(settled, 720, 260, 'Segoe UI', measure), `plot:${hoverHour}`);
    assert.ok((afterLast?.payload?.items ?? []).some(item => item.value !== undefined), `after the last kept change the figures stand: ${JSON.stringify(afterLast?.payload?.items)}`);

    const { subpaths } = buildLineSubpaths(lineModel.series[0]!, layout, H1941);
    const ys = subpaths.flat().map(p => p[1]);
    assert.ok(ys.some(y => Math.abs(y - layout.toY(40)) < 0.5), 'painter includes max excursion 40%');
    assert.ok(ys.some(y => Math.abs(y - layout.toY(10)) < 0.5), 'painter includes min excursion 10%');

    const hoverReply = {
      tensionScale: 1000,
      contributionScale: 100000,
      ceilingRaw: '10000000',
      unnamed: false,
      contributions: [{ slot: 1, tag: 'GER', remembered: false, reading: { Ok: '3500000' } }],
      sources: [],
      missing: null,
      warnings: [],
      live: {
        groups: [{ table: 3, columns: [0], keys: ['1'], values: [3500000] }],
        rows: ['1'],
        kinds: [{ id: 'value', op: 'value', scale: 1 }],
        cells: [0, 0, 1, 0, 0, 0],
      },
    };
    const exactView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', downsampledReply]]),
      hovers: new Map([['hover', { hour: hoverHour, data: hoverReply }]]),
      identity: makeIdentity(),
    });
    const exactModel = worldTensionLinesProjection.project(exactView, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    const exactLayout = layoutTimeChart(exactModel, 720, 260, 'Segoe UI', measure);
    const exactTip = deriveTimeChartTooltip(exactModel, exactLayout, `plot:${hoverHour}`);
    const exactItem = exactTip?.payload?.items?.find(i => i.label === 'German Reich');
    assert.equal(exactItem?.value, '+35%', 'exact hover overrides downsampled range with the signed slot contribution');
    const readStack = worldTensionProjection.project(createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', { ...downsampledReply, bands: [{ ...downsampledReply.bands[0]!, valuesRaw: ['0', '2000000', '6000000'] }] }]]),
      hovers: new Map([['hover', { hour: hoverHour, data: hoverReply }]]),
      identity: makeIdentity(),
    }), { extent: [H1936, H1941], surface: '#111111', layers: 5, values: 'share', showMarkers: false, gaps: [] });
    const readTip = deriveTimeChartTooltip(readStack, layoutTimeChart(readStack, 720, 260, 'Segoe UI', measure), `plot:${hoverHour}`);
    assert.ok(!(readTip?.payload?.items ?? []).some(item => item.missingReason?.includes('grouped span')), `once the hour is read its figures replace the grouped-span note: ${JSON.stringify(readTip?.payload?.items)}`);

    const plan = worldTensionLinesDeclaration.sources({
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1', firstHour: H1936, lastHour: H1941, gaps: [] },
      countries: { all: true, tags: [], top: null },
      options: {},
      widget: { countries: { kind: 'all' } },
      gateSpan: (a: number, b: number) => ({ firstHour: a, lastHour: b }),
    } as never, { width: 800, height: 300, following: false, staticSpan: null });
    assert.ok(plan.entries.some(e => e.name === 'hover' && e.read.shape === 'exact-hover'), 'lines view declares exact-hover entry');
    assert.equal('layers' in (plan.entries[0]!.read as { params: object }).params, false, 'All lines has no layer limit');
  });

  it('S14 + R3-S11: keys world-tension bands and audit rows by slot (even when two slots share a tag), and includes secondary country and place in events table rowKey', () => {
    const sharedTagReply = {
      ...sampleStack(),
      bands: [
        { slot: 1, tag: 'GER', remembered: true, holders: held('GER', true), values: [0, 20, 30], minValues: [], maxValues: [], endValue: 30, endRaw: '3000000', endMissing: null },
        { slot: 2, tag: 'GER', remembered: false, holders: held('GER'), values: [0, 10, 40], minValues: [], maxValues: [], endValue: 40, endRaw: '4000000', endMissing: null },
      ],
      other: null,
      events: [
        { hour: H1936 + 5000, country: 9001, slot: 1, secondCountry: 2, place: 0, tag: 'GER', remembered: false, reason: 'Declared war', addedRaw: '500000', firstRead: H1936 + 5000, lastRead: null },
        { hour: H1936 + 5000, country: 9001, slot: 1, secondCountry: 3, place: 0, tag: 'GER', remembered: false, reason: 'Declared war', addedRaw: '400000', firstRead: H1936 + 5000, lastRead: null },
        { hour: H1936 + 5000, country: 9001, slot: 1, secondCountry: 3, place: 1, tag: 'GER', remembered: false, reason: 'Declared war', addedRaw: '300000', firstRead: H1936 + 5000, lastRead: null },
        { hour: H1936 + 5000, country: 9050, slot: null, secondCountry: 3, place: 0, tag: null, remembered: false, reason: 'Declared war', addedRaw: '200000', firstRead: H1936 + 5000, lastRead: null },
        { hour: H1936 + 5000, country: 9051, slot: null, secondCountry: 3, place: 0, tag: null, remembered: false, reason: 'Declared war', addedRaw: '100000', firstRead: H1936 + 5000, lastRead: null },
      ],
    } as unknown as WorldTensionStackReply;

    const view = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', sharedTagReply]]),
      identity: makeIdentity(),
    });
    const stackModel = worldTensionProjection.project(view, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'recorded',
      showMarkers: false,
      gaps: [],
    });
    assert.deepEqual(stackModel.stack?.bands.map(b => b.key), ['slot:1', 'slot:2'], 'stack bands key by slot, not tag');
    const stackAuditKeys = (stackModel.auditRows ?? []).map(r => r.row);
    assert.equal(new Set(stackAuditKeys).size, stackAuditKeys.length, 'R3-S11: audit keys must be unique when two slots share a tag');
    assert.ok(stackAuditKeys.includes('slot:1') && stackAuditKeys.includes('slot:2'), 'R3-S11: stack audit keys use slot ids');

    const linesModel = worldTensionLinesProjection.project(view, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    assert.deepEqual(linesModel.series.map(s => s.key), ['slot:1', 'slot:2'], 'lines key by slot, not tag');
    assert.deepEqual(linesModel.series.map(s => s.id), ['slot:1', 'slot:2'], 'lines id by slot, not tag');
    const linesAuditKeys = (linesModel.auditRows ?? []).map(r => r.row);
    assert.deepEqual(linesAuditKeys, ['slot:1', 'slot:2'], 'R3-S11: lines audit keys use slot ids');

    const table = worldTensionEventsProjection.project(view, { branchFirst: H1936 });
    const rowKeys = table.rows.map(r => r.key);
    assert.equal(new Set(rowKeys).size, rowKeys.length, 'events table rowKey distinguishes secondary country, occurrence ordinal and unresolved primary country');
  });

  it('S15 + S18: passes top and dated GroupSelection to Rust across Stack/Lines/Events, renders refused replies, and skips unused stack/event work', () => {
    const viewReq = { width: 800, height: 300, following: false, staticSpan: null };
    const mkCtx = (countries: unknown, widgetCountries: unknown, options: Record<string, unknown> = {}) => ({
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1', firstHour: H1936, lastHour: H1941, gaps: [] },
      countries,
      options,
      widget: { countries: widgetCountries },
      gateSpan: (a: number, b: number) => ({ firstHour: a, lastHour: b }),
    } as never);

    const topCtx = mkCtx({ all: false, tags: [], top: 3 }, { kind: 'top', count: 3 });
    const playing = worldTensionDeclaration.sources(topCtx, { ...viewReq, following: true });
    assert.deepEqual(playing.entries.filter(entry => entry.read.shape === 'point' && entry.read.followQuery)
      .map(entry => entry.name), ['history', 'main']);
    const stackTopParams = (worldTensionDeclaration.sources(topCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    const linesTopParams = (worldTensionLinesDeclaration.sources(topCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    const eventsTopParams = (worldTensionEventsDeclaration.sources(topCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    assert.equal(stackTopParams.top, 3, 'Stack passes Top-N count to product');
    assert.equal(stackTopParams.countries, null, 'Stack does not send empty countries array for Top-N');
    assert.equal(linesTopParams.top, 3, 'Lines passes Top-N count to product');
    assert.equal(eventsTopParams.top, 3, 'Events passes Top-N count to product');
    assert.equal(eventsTopParams.includeStack, false, 'S18: Events skips building stack history');
    assert.equal(linesTopParams.includeEvents, false, 'S18: Lines skips scanning events');

    const noMarkersCtx = mkCtx({ all: true, tags: [], top: null }, { kind: 'all' }, { markers: false });
    const stackNoMarkers = (worldTensionDeclaration.sources(noMarkersCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    assert.equal(stackNoMarkers.includeEvents, false, 'S18: Stack with markers off skips scanning events');

    const groupCtx = mkCtx(
      { all: false, tags: ['GER', 'ITA'], top: null },
      { kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour' },
    );
    const expectedGroup = { type: 'faction', id: '42', membership: 'hour' };
    const stackGroupParams = (worldTensionDeclaration.sources(groupCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    const linesGroupParams = (worldTensionLinesDeclaration.sources(groupCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    const eventsGroupParams = (worldTensionEventsDeclaration.sources(groupCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    assert.deepEqual(stackGroupParams.group, expectedGroup, 'Stack sends GroupSelection directly');
    assert.equal(stackGroupParams.countries, null, 'Stack does not flatten faction group to countries union');
    assert.deepEqual(linesGroupParams.group, expectedGroup, 'Lines sends GroupSelection directly');
    assert.deepEqual(eventsGroupParams.group, expectedGroup, 'Events sends GroupSelection directly');

    const refusedReply = {
      ...sampleStack(),
      refused: 'Membership of this group is not recorded at this hour',
      hours: [],
      total: [],
      bands: [],
      other: null,
      events: [],
    } as unknown as WorldTensionStackReply;
    const refusedView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', refusedReply]]),
      identity: makeIdentity(),
    });
    const refusedStack = worldTensionProjection.project(refusedView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: true,
      gaps: [],
    });
    assert.equal(refusedStack.empty?.kind, 'unavailable');
    assert.equal(refusedStack.empty?.message, 'Membership of this group is not recorded at this hour');
    const refusedLines = worldTensionLinesProjection.project(refusedView, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    assert.equal(refusedLines.empty?.kind, 'unavailable');
    assert.equal(refusedLines.empty?.message, 'Membership of this group is not recorded at this hour');
    const refusedEvents = worldTensionEventsProjection.project(refusedView, { branchFirst: H1936 });
    assert.equal(refusedEvents.emptyKind, 'unavailable');
    assert.equal(refusedEvents.empty, 'Membership of this group is not recorded at this hour');
  });

  it('formats total, band shares, Other, lines, tooltips, and audit from raw integers without float drift', () => {
    const driftFloat = (145 / 700) * 70;
    const shareReply = {
      ...sampleStack(),
      cappedEndTension: 70,
      cappedEndTensionRaw: '70000',
      endSumRaw: '700',
      hours: [H1936, H1941],
      total: [0, 70],
      totalRaw: ['0', '70000'],
      sumRaw: ['0', '700'],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, holders: held('GER'), values: [0, driftFloat], valuesRaw: ['0', '145'], minValues: [], maxValues: [], endValue: driftFloat, endRaw: '145', endMissing: null },
        { slot: 2, tag: 'JAP', remembered: false, holders: held('JAP'), values: [0, 41], valuesRaw: ['0', '410'], minValues: [], maxValues: [], endValue: 41, endRaw: '410', endMissing: null },
      ],
      other: { count: 4, values: [0, driftFloat], valuesRaw: ['0', '145'], endValue: driftFloat, endRaw: '145', endMissing: null },
      events: [],
    } as unknown as WorldTensionStackReply;
    const shareModel = worldTensionProjection.project(
      createBoundedReadView({ hour: H1941, panels: new Map([['main', shareReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1941], surface: '#111111', layers: 5, values: 'share', showMarkers: false, gaps: [] },
    );
    assert.equal(shareModel.stack?.bands[0]?.endLabel?.value, '+14.5%', '145/700 of 70% keeps the signed contribution figure on the end label');
    assert.equal(shareModel.stack?.bands.at(-1)?.endLabel?.value, '+14.5%', 'Other 145/700 of 70% keeps the signed contribution figure on the end label');
    assert.equal(shareModel.auditRows?.find(r => r.row === 'slot:1')?.value, '+14.5%');
    assert.equal(shareModel.auditRows?.find(r => r.row === 'other')?.value, '+14.5%');
    const shareLayout = layoutTimeChart(shareModel, 960, 280, 'Segoe UI', measure);
    const shareTip = deriveTimeChartTooltip(shareModel, shareLayout, 'label:__total__');
    assert.equal(shareTip?.payload?.items?.find(i => i.label === 'German Reich')?.value, '+14.5%');
    assert.equal(shareTip?.payload?.items?.find(i => i.label === 'Other (4)')?.value, '+14.5%');

    const bigRaw = '999999999999999999';
    const bigFloat = Number(bigRaw) / 100_000;
    const recReply = {
      ...sampleStack(),
      cappedEndTension: 100,
      cappedEndTensionRaw: '100000',
      endSumRaw: bigRaw,
      hours: [H1941],
      total: [100],
      totalRaw: ['100000'],
      sumRaw: [bigRaw],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, holders: held('GER'), values: [bigFloat], valuesRaw: [bigRaw], minValues: [], maxValues: [], endValue: bigFloat, endRaw: bigRaw, endMissing: null },
      ],
      other: { count: 1, values: [bigFloat], valuesRaw: [bigRaw], endValue: bigFloat, endRaw: bigRaw, endMissing: null },
      events: [],
    } as unknown as WorldTensionStackReply;
    const recModel = worldTensionProjection.project(
      createBoundedReadView({ hour: H1941, panels: new Map([['main', recReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1941], surface: '#111111', layers: 5, values: 'recorded', showMarkers: false, gaps: [] },
    );
    assert.equal(recModel.stack?.total?.endLabel?.value, '100%', 'the world line is the recorded world tension');
    assert.equal(recModel.stack?.bands[0]?.endLabel?.value, '+9,999,999,999,999.99999%');
    assert.equal(recModel.stack?.bands[1]?.endLabel?.value, '+9,999,999,999,999.99999%');
    assert.equal(recModel.auditRows?.find(r => r.row === '__total__')?.value, '100%');
    assert.equal(recModel.auditRows?.find(r => r.row === 'other')?.value, '+9,999,999,999,999.99999%');
    const recLayout = layoutTimeChart(recModel, 960, 280, 'Segoe UI', measure);
    const recTip = deriveTimeChartTooltip(recModel, recLayout, 'label:__total__');
    assert.equal(recTip?.payload?.items?.[0]?.value, '100%');
    assert.equal(recTip?.payload?.items?.find(i => i.label === 'German Reich')?.value, '+9,999,999,999,999.99999%');

    const linesModel = worldTensionLinesProjection.project(
      createBoundedReadView({ hour: H1941, panels: new Map([['main', recReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1941], surface: '#111111', gaps: [] },
    );
    assert.equal(linesModel.series[0]?.endLabel?.value, '+9,999,999,999,999.99999%');
    assert.equal(linesModel.auditRows?.find(r => r.row === 'slot:1')?.value, '+9,999,999,999,999.99999%');
    const linesLayout = layoutTimeChart(linesModel, 960, 280, 'Segoe UI', measure);
    const linesTip = deriveTimeChartTooltip(linesModel, linesLayout, `plot:${H1941}`);
    assert.equal(linesTip?.payload?.items?.find(i => i.label === 'German Reich')?.value, '+9,999,999,999,999.99999%');
  });

  it('R2-S11: through the manifest entry, the world label and audit are the recorded world tension in both modes, a negative share keeps its label, and every band, Other and line says its value or why it is missing', async () => {
    const at = H1941;
    const band = (slot: number, tag: string | null, raw: string | null, missing: string | null) => ({
      slot, tag, remembered: false, holders: tag ? held(tag) : [],
      values: [raw === null ? null : Number(raw) / 100000], valuesRaw: [raw], minValues: [], maxValues: [],
      endValue: raw === null ? null : Number(raw) / 100000, endRaw: raw, endMissing: missing,
    });
    const tails = Array.from({ length: 8 }, (_, i) => band(10 + i, null, i === 7 ? null : String(100000 * (8 - i)), i === 7 ? 'This country\'s threat contribution was not recorded at this hour.' : null));
    const stack: WorldTensionStackReply = {
      ...sampleStack(),
      endHour: at,
      cappedEndTension: 100,
      cappedEndTensionRaw: '100000',
      endMissing: null,
      endSumRaw: '12500000',
      hours: [at],
      total: [100],
      totalRaw: ['100000'],
      sumRaw: ['12500000'],
      bands: [band(1, 'GER', '13000000', null), band(3, 'ITA', '-500000', null), ...tails],
      other: { count: 2, values: [null], valuesRaw: [null], endValue: null, endRaw: null, endMissing: 'A shown country\'s contribution was not recorded at this hour, so the rest is not known.' },
      events: [],
      nextChangeHour: null,
    };
    const hover: WorldTensionReply = { tensionScale: 1000, contributionScale: 100000, ceilingRaw: '10000000', unnamed: false, contributions: [], sources: [], missing: null, warnings: [] };
    const client = {
      product: async (request: { product: string }) => ({ data: request.product === 'world-tension-stack' ? stack : hover }),
    } as unknown as DashboardClient;
    const gate: FrozenGate = { recorded: (h: number | null | undefined) => (typeof h === 'number' ? h : null) as never, span: (firstHour: number, lastHour: number) => ({ firstHour, lastHour } as never) };
    const project = async (view: string, values: 'share' | 'recorded') => {
      const entry = FAMILY_VIEWS.find(item => item.kind === 'world-tension' && item.view === view)!;
      const spec = await entry.loadSpec();
      const declaration = spec.declaration as TimeChartWidgetDeclaration<unknown>;
      const ctx = {
        track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
        branch: { id: '1', firstHour: H1936, lastHour: at, gaps: [] },
        countries: { all: true, tags: [], top: null },
        options: { values, markers: false },
        widget: { countries: { kind: 'all' } },
      } as unknown as FamilyPlanContext;
      const plan = declaration.sources(ctx, { width: 960, height: 280, following: false, staticSpan: null });
      const readView = await readPlanExact(plan, at, { client, following: false, timeMode: 'pin', branch: { firstHour: H1936, lastHour: at }, window: null, staticSpan: [H1936, at], gate, identity: makeIdentity() });
      assert.ok(readView, view);
      return spec.projection.project(readView!, declaration.projectionOptions(ctx, { extent: [H1936, at], surface: '#111111' })) as TimeChartModel;
    };
    const audit = (model: TimeChartModel) => new Map((model.auditRows ?? []).map(r => [r.row, r.value]));

    const share = await project('tension', 'share');
    assert.equal(share.stack?.total?.endLabel?.value, '100%', 'the world label is the recorded world tension, never the 125% sum of contributions');
    assert.equal(audit(share).get('__total__'), '100%');
    const italy = share.stack?.bands.find(b => b.id === 'slot:3');
    assert.equal(italy?.endLabel?.value, '−4%', 'a negative share keeps its end label');
    assert.equal(audit(share).get('slot:3'), '−4%');
    assert.equal(audit(share).get('slot:17'), 'This country\'s threat contribution was not recorded at this hour.');
    assert.equal(audit(share).get('other'), 'A shown country\'s contribution was not recorded at this hour, so the rest is not known.');
    assert.equal(audit(share).size, 12, 'the world, every band and Other');

    const recorded = await project('tension', 'recorded');
    assert.equal(recorded.stack?.total?.endLabel?.value, '100%', 'in recorded mode too the world line is the recorded world tension');
    assert.equal(audit(recorded).get('__total__'), '100%');
    assert.equal(audit(recorded).get('slot:1'), '+130%');

    const lines = await project('lines', 'recorded');
    assert.equal(lines.series.filter(s => s.role === 'tail').length, 2);
    assert.equal(audit(lines).size, lines.series.length, 'every line is audited, tail lines included');
    assert.equal(audit(lines).get('slot:16'), '+2%');
    assert.equal(audit(lines).get('slot:17'), 'This country\'s threat contribution was not recorded at this hour.');
  });
});
