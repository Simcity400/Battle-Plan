import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import { boundTiledSeries, createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { readPlanExact } from '../../families/broker.ts';
import { freezeGate } from '../../families/sources.ts';
import { IdentityIndex } from '../../families/identity/index.ts';
import { createSnapshotIdentityProvider } from '../../families/identity/provider.ts';
import { readyArt } from '../../families/art/types.ts';
import { deriveTimeChartTooltip } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import type { ApmReply } from '../../generated/dashboard';
import { apmTableProjection } from '../player-apm/apmProjection.ts';
import { apmGraphDeclaration, apmGraphProjection } from './apmGraphProjection.ts';

const H1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
const H1943 = engineHour({ year: 1943, month: 9, day: 7, hour: 1 })!;

function makeTiled(id: string, val: number, lastHour = H1943): TiledLine {
  return {
    tag: id,
    head: 'h',
    decimals: 0,
    summary: {
      from: H1936,
      to: lastHour,
      width: 2,
      buckets: [{
        firstHour: H1936, lastHour, firstValue: val, lastValue: val, lastValueHour: lastHour,
        minHour: H1936, minValue: val, maxHour: lastHour, maxValue: val, firstBreak: false, hasBreak: false,
      }],
      tiles: { binHours: 96, valueAt: () => null },
    },
    missing: [],
    parts: [],
    marks: null,
  };
}

describe('apm-graph projection', () => {
  it('keeps historical duplicate-name suffixes when the exact hover arrives', () => {
    const first = H1936, middle = first + 5, last = H1943;
    const run = (firstHour: number, lastHour: number, name: string) => ({ firstHour, lastHour, tag: 'AAA', name, nameMissing: null });
    const players = [
      { id: 'user:10', name: 'Shared', nameMissing: null, tags: ['AAA'], runs: [run(first, last, 'Shared')] },
      { id: 'user:2', name: 'Later', nameMissing: null, tags: ['AAA'], runs: [
        run(first, middle - 1, 'Earlier'), run(middle, middle, 'Shared'), run(middle + 1, last, 'Later'),
      ] },
    ];
    const excluded = { id: 'user:1', name: 'Shared', nameMissing: null, tags: ['BBB'],
      runs: [{ ...run(first, last, 'Shared'), tag: 'BBB' }] };
    const departed = { id: 'user:3', name: 'Shared', nameMissing: null, tags: ['AAA'], runs: [run(first, first, 'Shared')] };
    const options = apmGraphDeclaration.projectionOptions({
      options: { group: 'player' }, countries: { all: false, tags: ['AAA'] },
      aux: { available: true, players: [...players, excluded, departed] }, branch: { gaps: [] },
    } as never, { extent: [first, last], surface: '#111111' });
    const tiles = players.map((p, i) => [p.id, boundTiledSeries(makeTiled(p.id, 8 + i), displayedHour(last))] as const);
    tiles.push([departed.id, boundTiledSeries(makeTiled(departed.id, 7, first), displayedHour(last))]);
    const ids = players.map(p => p.id);
    const replyAt = (hour: number, replyIds: string[]): ApmReply => ({
      available: true, players: [], warnings: [], series: players.slice(0, replyIds.length).map((p, i) => ({
        id: replyIds[i]!, name: p.runs.find(r => r.firstHour <= hour && hour <= r.lastHour)!.name, nameMissing: null, tag: 'AAA', hour,
        summary: { apm: 8 + i, confirmedApm: 8 + i, orders: String(8 + i), confirmedOrders: String(8 + i), pausedOrders: '0',
          unpausedMinutes: 1, unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: false }, points: [],
      })),
    });
    const project = (hour: number, exact: boolean, exactIds = ids) => apmGraphProjection.project(createBoundedReadView({
      hour: last, series: new Map([['apm', new Map(tiles)]]), panels: new Map([['head', replyAt(last, ids)]]),
      hovers: exact ? new Map([['hover', { hour, data: replyAt(hour, exactIds) }]]) : undefined,
    }), options);
    const labels = (hour: number, exact: boolean, exactIds = ids) => {
      const model = project(hour, exact, exactIds);
      const layout = layoutTimeChart(model, 600, 260, 'sans-serif', text => text.length * 6);
      const items = deriveTimeChartTooltip(model, layout, `plot:${hour}`)?.payload?.items ?? [];
      return Object.fromEntries(items.filter(item => item.value).map(item => [item.value, item.label]));
    };
    const head = project(last, false).series.find(s => s.id === 'user:10');
    assert.equal(head?.endLabel?.fullName, 'Shared');
    for (const [hour, expected] of [
      [middle - 1, { 8: 'Shared', 9: 'Earlier' }],
      [middle, { 8: 'Shared (2)', 9: 'Shared (1)' }],
      [middle + 1, { 8: 'Shared', 9: 'Later' }],
    ] as const) {
      assert.deepEqual(labels(hour, false), expected);
      assert.deepEqual(labels(hour, true), expected);
    }
    assert.deepEqual(labels(middle, true, ['user:10']), { 8: 'Shared (2)' });
    assert.deepEqual(labels(middle, true, ['user:0', 'user:2']), { 8: 'Shared (3)', 9: 'Shared (1)' });
  });

  it('numbers a head-only duplicate player together with registered players until the registry catches up (#1063)', () => {
    const first = H1936, at = first + 5;
    const run = (firstHour: number, lastHour: number, name = 'Shared') => ({ firstHour, lastHour, tag: 'AAA', name, nameMissing: null });
    const reg2 = [
      { id: 'user:2', name: 'Shared', nameMissing: null, tags: ['AAA'], runs: [run(first, at)] },
      { id: 'user:10', name: 'Shared', nameMissing: null, tags: ['AAA'], runs: [run(first, at)] },
    ];
    const reg2Lagged = [
      { id: 'user:2', name: 'Shared', nameMissing: null, tags: ['AAA'], runs: [run(first, at - 1)] },
      { id: 'user:10', name: 'Shared', nameMissing: null, tags: ['AAA'], runs: [run(first, at - 1)] },
    ];
    const reg3 = [
      { id: 'user:0', name: 'Shared', nameMissing: null, tags: ['AAA'], runs: [run(at, at)] },
      ...reg2,
    ];
    const makeOpts = (players: typeof reg2) => apmGraphDeclaration.projectionOptions({
      options: { group: 'player' }, countries: { all: false, tags: ['AAA'] },
      aux: { available: true, players }, branch: { gaps: [] },
    } as never, { extent: [first, at], surface: '#111111' });
    const rows = [
      { id: 'user:0', apm: 30 },
      { id: 'user:2', apm: 20 },
      { id: 'user:10', apm: 10 },
    ];
    const reply: ApmReply = {
      available: true, players: [], warnings: [],
      series: rows.map(({ id, apm }) => ({
        id, name: 'Shared', nameMissing: null, tag: 'AAA', hour: at, points: [],
        summary: { apm, confirmedApm: apm, orders: String(apm), confirmedOrders: String(apm), pausedOrders: '0',
          unpausedMinutes: 1, unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: false },
      })),
    };
    for (const players of [reg2, reg2Lagged, reg3]) {
      const registeredIds = new Set(players.map(p => p.id));
      const tiles = new Map(rows
        .filter(({ id }) => registeredIds.has(id))
        .map(({ id, apm }) => {
          const tiled = makeTiled(id, apm, at);
          if (id === 'user:0') {
            const bucket = tiled.summary.buckets[0]!;
            tiled.summary = { ...tiled.summary, from: at, buckets: [{ ...bucket, firstHour: at, minHour: at }] };
          }
          return [id, boundTiledSeries(tiled, displayedHour(at))];
        }));
      const options = makeOpts(players);
      const project = (exact: boolean) => apmGraphProjection.project(createBoundedReadView({
        hour: at, series: new Map([['apm', tiles]]), panels: new Map([['head', reply]]),
        hovers: exact ? new Map([['hover', { hour: at, data: reply }]]) : undefined,
      }), options);
      const plain = project(false);
      assert.deepEqual(
        Object.fromEntries(plain.series.map(s => [s.id, s.endLabel?.fullName])),
        { 'user:0': 'Shared (1)', 'user:2': 'Shared (2)', 'user:10': 'Shared (3)' },
      );
      for (const exact of [false, true]) {
        const model = project(exact);
        const layout = layoutTimeChart(model, 600, 260, 'sans-serif', text => text.length * 6);
        const items = deriveTimeChartTooltip(model, layout, `plot:${at}`)?.payload?.items ?? [];
        const byValue = Object.fromEntries(items.filter(item => item.value).map(item => [item.value, item.label]));
        assert.deepEqual(byValue, { 30: 'Shared (1)', 20: 'Shared (2)', 10: 'Shared (3)' });
      }
      const earlierLayout = layoutTimeChart(plain, 600, 260, 'sans-serif', text => text.length * 6);
      const earlierItems = deriveTimeChartTooltip(plain, earlierLayout, `plot:${first}`)?.payload?.items ?? [];
      const earlierByValue = Object.fromEntries(earlierItems.filter(item => item.value).map(item => [item.value, item.label]));
      assert.deepEqual(earlierByValue, { 20: 'Shared (1)', 10: 'Shared (2)' });
    }
  });

  it('uses the player and country recorded at the hovered hour for the name, flag and swatch', () => {
    const id = 'user:1', other = 'user:2';
    const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: 'b', content: 0 }, [H1936, H1943]);
    index.setFacts({
      branchId: 'b', known: [[H1936, H1943]], names: ['North', 'South'], reasons: [], warnings: [],
      flags: ['north', 'south'].map(key => ({ keys: [key, null, null], reasons: [null, null, null] })),
      countries: [{ tag: 'AAA', runs: [[H1936, 0, 0xaa5030, -1, 0]] }, { tag: 'BBB', runs: [[H1936, 1, 0x3070cc, -1, 1]] }],
    });
    const tiled = makeTiled(id, 8);
    const paused = 'No unpaused time was observed in this period.';
    const bounded = boundTiledSeries({ ...tiled, missing: [{ firstHour: H1936 + 2, lastHour: H1936 + 2, reason: paused }] }, displayedHour(H1943));
    const head: ApmReply = { available: true, players: [], warnings: [], series: [{
      id, tag: 'BBB', name: null, nameMissing: 'Name not recorded', hour: H1943, points: [],
      summary: { apm: 8, confirmedApm: 8, orders: '8', confirmedOrders: '8', pausedOrders: '0', unpausedMinutes: 1,
        unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: false },
    }] };
    const view = createBoundedReadView({
      hour: H1943, series: new Map([['apm', new Map([[id, bounded], [other, bounded]])]]), identity: createSnapshotIdentityProvider(index),
      panels: new Map([['head', head]]), hovers: new Map([['hover', { hour: H1943, data: head }]]),
      art: { resolve: handle => handle.kind === 'flag' ? readyArt(handle.choice) : { status: 'pending' } },
    });
    const model = apmGraphProjection.project(view, {
      group: 'player', extent: [H1936, H1943], surface: '#111111', gaps: [], yMin: null, yMax: null, available: true,
      players: [{ id, name: 'Later', nameMissing: null, tags: ['AAA', 'BBB'], runs: [
        { firstHour: H1936, lastHour: H1936 + 5, tag: 'AAA', name: 'Earlier', nameMissing: null },
        { firstHour: H1936 + 6, lastHour: H1943 - 1, tag: 'BBB', name: 'Later', nameMissing: null },
        { firstHour: H1943, lastHour: H1943, tag: 'BBB', name: null, nameMissing: 'Name not recorded' },
      ] }, { id: other, name: 'Later', nameMissing: null, tags: ['AAA'], runs: [
        { firstHour: H1936 + 6, lastHour: H1943, tag: 'AAA', name: 'Later', nameMissing: null },
      ] }],
    });
    const layout = layoutTimeChart(model, 600, 260, 'sans-serif', text => text.length * 6);
    const items = (hour: number) => deriveTimeChartTooltip(model, layout, `plot:${hour}`)?.payload?.items ?? [];
    const item = (hour: number) => items(hour)[0];
    assert.equal(item(H1936 + 2)?.missingReason, paused);
    assert.equal(item(H1936 + 2)?.value, undefined);
    const early = item(H1936 + 1);
    assert.equal(early?.label, 'Earlier · North');
    assert.match(early?.countryFlag?.flag ?? '', /north/);
    assert.deepEqual(items(H1936 + 6).map(row => row.label).sort(), ['Later (1) · South', 'Later (2) · North']);
    assert.notEqual(early?.swatch, model.series[0]?.color);
    const endLabel = model.series.find(s => s.id === id)?.endLabel;
    assert.equal(item(H1943)?.label, 'Name not recorded · South');
    assert.equal(endLabel?.name, item(H1943)?.label);
    assert.equal(endLabel?.fullName, item(H1943)?.label);
  });
  it('shows named unavailable state on single-player recordings with no player orders', () => {
    const readView = createBoundedReadView({ hour: H1943 });
    const model = apmGraphProjection.project(readView, {
      group: 'player',
      extent: [H1936, H1943],
      surface: '#111111',
      gaps: [],
      yMin: null,
      yMax: null,
      available: false,
    });
    assert.equal(model.empty?.kind, 'unavailable');
    assert.equal(model.empty?.message, 'Player orders were not recorded in this campaign.');
  });

  it('treats a recorded channel with zero eligible players as an empty state matching the table, never as not recorded', () => {
    const makeCtx = (available: boolean) => ({
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1', firstHour: H1936, lastHour: H1943, gaps: [] },
      countries: { all: true, tags: [], top: null },
      options: { group: 'player' },
      aux: { available, players: [], series: [], warnings: [] } satisfies ApmReply,
      gateSpan: (a: number, b: number) => ({ firstHour: a, lastHour: b }),
    });
    const recordedCtx = makeCtx(true);
    const unrecordedCtx = makeCtx(false);

    assert.equal(apmGraphDeclaration.unavailable?.(recordedCtx as never, H1943), null);
    const recordedPlan = apmGraphDeclaration.sources(recordedCtx as never, { width: 800, height: 300, following: true, staticSpan: null });
    assert.equal(recordedPlan.unavailable, undefined);
    assert.ok(recordedPlan.entries.some(e => e.name === 'head'));

    assert.equal(apmGraphDeclaration.unavailable?.(unrecordedCtx as never, H1943), 'Player orders were not recorded in this campaign.');
    const unrecordedPlan = apmGraphDeclaration.sources(unrecordedCtx as never, { width: 800, height: 300, following: true, staticSpan: null });
    assert.equal(unrecordedPlan.unavailable, 'Player orders were not recorded in this campaign.');
    assert.deepEqual(unrecordedPlan.entries, []);

    const emptyReply: ApmReply = { available: true, players: [], series: [], warnings: [] };
    const graphOpts = apmGraphDeclaration.projectionOptions(recordedCtx as never, { extent: [H1936, H1943], surface: '#111111' });
    const graphModel = apmGraphProjection.project(createBoundedReadView({ hour: H1943, panels: new Map([['head', emptyReply]]) }), graphOpts);
    const tableModel = apmTableProjection.project(createBoundedReadView({ hour: H1943, panels: new Map([['main', emptyReply]]) }), {
      players: true, showPaused: false, rankMarkers: false,
    });
    assert.equal(graphModel.empty?.kind, 'empty');
    assert.equal(tableModel.emptyKind, 'empty');
    assert.equal(graphModel.empty?.message, 'No eAPM is available for these countries, players and dates.');
    assert.equal(graphModel.empty?.message, tableModel.empty);

    const unrecordedReply: ApmReply = { available: false, players: [], series: [], warnings: [] };
    const tableUnavailable = apmTableProjection.project(createBoundedReadView({ hour: H1943, panels: new Map([['main', unrecordedReply]]) }), {
      players: true, showPaused: false, rankMarkers: false,
    });
    assert.equal(tableUnavailable.emptyKind, 'unavailable');
    assert.equal(tableUnavailable.empty, 'Player orders were not recorded in this campaign.');
  });

  it('draws top 8 ranked players with end labels and flags, and ranks 9+ as grey tail lines', () => {
    const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: 'b', content: 0 }, [H1936, H1943]);
    index.setFacts({
      branchId: 'b',
      known: [[H1936, H1943]],
      names: ['Italy'],
      reasons: [],
      flags: [{ keys: ['ITA_fascism', null, null], reasons: [null, null, null] }],
      countries: [{ tag: 'ITA', runs: [[H1936, 0, 0x3b8e4a, -1, 0]] }],
      warnings: [],
    });
    const series = Array.from({ length: 10 }, (_, i) => ({
      id: `user:${i}`,
      name: `Player ${i + 1}`,
      nameMissing: null,
      tag: 'ITA',
      hour: H1943,
      summary: { apm: 100 - i * 8, confirmedApm: 100 - i * 8, orders: String((100 - i * 8) * 10), confirmedOrders: String((100 - i * 8) * 10), pausedOrders: '5', unpausedMinutes: 10, unpausedMinutesNum: '10', unpausedMinutesDen: '1', complete: true, gaps: false },
      points: [],
    }));
    const head: ApmReply = { available: true, players: [], series, warnings: [] };
    const tiledMap = new Map(series.map(s => [s.id, boundTiledSeries(makeTiled(s.id, s.summary.apm!), displayedHour(H1943))]));
    const readView = createBoundedReadView({
      hour: H1943,
      panels: new Map([['head', head]]),
      series: new Map([['apm', tiledMap]]),
      identity: createSnapshotIdentityProvider(index),
    });
    const model = apmGraphProjection.project(readView, {
      group: 'player',
      extent: [H1936, H1943],
      surface: '#111111',
      gaps: [],
      yMin: null,
      yMax: null,
      available: true,
    });
    assert.equal(model.series.length, 10);
    assert.equal(model.series.filter(s => s.role === 'primary').length, 8);
    assert.equal(model.series.filter(s => s.role === 'tail').length, 2);
    assert.equal(model.series[0]?.endLabel?.name, 'Player 1');
    assert.equal(model.series[0]?.endLabel?.value, '100');
    assert.equal(model.series[8]?.endLabel, null);
    assert.equal(model.auditRows?.length, 8);
  });

  it('S7: retains departed players from tiles, surfaces product gap flag, and uses exact hovered-hour rates', () => {
    const departedId = 'user:departed';
    const activeId = 'user:active';
    const head: ApmReply = {
      available: true,
      players: [],
      series: [{
        id: activeId,
        name: 'Active',
        nameMissing: null,
        tag: 'ITA',
        hour: H1943,
        summary: { apm: 42, confirmedApm: 42, orders: '42', confirmedOrders: '42', pausedOrders: '0', unpausedMinutes: 1, unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: true },
        points: [],
      }],
      warnings: [],
    };
    const hoverReply: ApmReply = {
      available: true,
      players: [],
      series: [{
        id: departedId,
        name: 'Departed',
        nameMissing: null,
        tag: 'GER',
        hour: H1936 + 1,
        summary: { apm: 20, confirmedApm: 20, orders: '20', confirmedOrders: '20', pausedOrders: '0', unpausedMinutes: 1, unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: true },
        points: [],
      }],
      warnings: [],
    };
    const tiledMap = new Map([
      [activeId, boundTiledSeries(makeTiled(activeId, 42), displayedHour(H1943))],
      [departedId, boundTiledSeries(makeTiled(departedId, 20), displayedHour(H1943))],
    ]);
    const readView = createBoundedReadView({
      hour: H1943,
      panels: new Map([['head', head]]),
      series: new Map([['apm', tiledMap]]),
      hovers: new Map([['hover', { hour: H1936 + 1, data: hoverReply }]]),
    });
    const model = apmGraphProjection.project(readView, {
      group: 'player',
      extent: [H1936, H1943],
      surface: '#111111',
      gaps: [],
      yMin: null,
      yMax: null,
      available: true,
      players: [
        { id: activeId, name: 'Active', nameMissing: null, tags: ['ITA'], runs: [{ firstHour: H1936, lastHour: H1943, tag: 'ITA', name: 'Active', nameMissing: null }] },
        { id: departedId, name: 'Departed', nameMissing: null, tags: ['GER'], runs: [{ firstHour: H1936, lastHour: H1943, tag: 'GER', name: 'Departed', nameMissing: null }] },
      ],
    } as Parameters<typeof apmGraphProjection.project>[1]);
    const departed = model.series.find(s => s.id === departedId);
    assert.ok(departed, 'departed player who left the trailing window keeps their recorded line');
    assert.equal(departed?.headValue, null);
    assert.equal(departed?.endLabel, null);
    assert.equal(departed?.name, 'Departed');
    const activeSeries = model.series.find(s => s.id === activeId);
    assert.equal(activeSeries?.endLabel?.valueMuted, true, 'gap flag marks end label muted');
    assert.ok(model.note?.includes('not recorded'), 'product gap flag surfaces in chart note');
    assert.ok(model.hoverDetail, 'exact hovered-hour reply builds hoverDetail');
    assert.equal(model.hoverDetail?.hour, H1936 + 1);
    assert.ok(model.hoverDetail?.items.some(item => item.label === 'Departed' && item.value === '20'));
    assert.ok(model.hoverDetail?.items.some(item => item.tone === 'muted' && item.label.includes('not recorded')));
  });

  it('S14: resolves historical APM hover and departed player identity at the drawn hour without leaking later names or tags', () => {
    const playerId = 'user:renamed';
    const departedId = 'user:departed';
    const midHour = H1936 + 100;
    const head: ApmReply = {
      available: true,
      players: [],
      series: [{
        id: playerId,
        name: 'LateName',
        nameMissing: null,
        tag: 'ITA',
        hour: H1943,
        summary: { apm: 50, confirmedApm: 50, orders: '50', confirmedOrders: '50', pausedOrders: '0', unpausedMinutes: 1, unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: false },
        points: [],
      }],
      warnings: [],
    };
    const hoverReply: ApmReply = {
      available: true,
      players: [],
      series: [{
        id: playerId,
        name: 'EarlyName',
        nameMissing: null,
        tag: 'GER',
        hour: H1936 + 5,
        summary: { apm: 30, confirmedApm: 30, orders: '30', confirmedOrders: '30', pausedOrders: '0', unpausedMinutes: 1, unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: false },
        points: [],
      }],
      warnings: [],
    };
    const tiledMap = new Map([
      [playerId, boundTiledSeries(makeTiled(playerId, 50), displayedHour(H1943))],
      [departedId, boundTiledSeries({
        ...makeTiled(departedId, 15),
        summary: {
          from: H1936,
          to: midHour,
          width: 1,
          buckets: [{
            firstHour: H1936, lastHour: midHour, firstValue: 15, lastValue: 15, lastValueHour: midHour,
            minHour: H1936, minValue: 15, maxHour: midHour, maxValue: 15, firstBreak: false, hasBreak: false,
          }],
          tiles: { binHours: 96, valueAt: () => null },
        },
      }, displayedHour(midHour + 50))],
    ]);
    const readView = createBoundedReadView({
      hour: midHour + 50,
      panels: new Map([['head', head]]),
      series: new Map([['apm', tiledMap]]),
      hovers: new Map([['hover', { hour: H1936 + 5, data: hoverReply }]]),
    });
    const model = apmGraphProjection.project(readView, {
      group: 'player',
      extent: [H1936, H1943],
      surface: '#111111',
      gaps: [],
      yMin: null,
      yMax: null,
      available: true,
      players: [
        {
          id: playerId,
          name: 'LateName',
          nameMissing: null,
          tags: ['GER', 'ITA'],
          runs: [
            { firstHour: H1936, lastHour: midHour, tag: 'GER', name: 'EarlyName', nameMissing: null },
            { firstHour: midHour + 100, lastHour: H1943, tag: 'ITA', name: 'LateName', nameMissing: null },
          ],
        },
        {
          id: departedId,
          name: 'FutureDepartedName',
          nameMissing: null,
          tags: ['FRA', 'ENG'],
          runs: [
            { firstHour: H1936, lastHour: midHour, tag: 'FRA', name: 'EarlyDepartedName', nameMissing: null },
            { firstHour: H1943 - 10, lastHour: H1943, tag: 'ENG', name: 'FutureDepartedName', nameMissing: null },
          ],
        },
      ] as unknown as ApmReply['players'],
    });
    assert.ok(
      model.hoverDetail?.items.some(item => item.label === 'EarlyName' && item.value === '30'),
      `hover at early hour must use EarlyName, not LateName: ${JSON.stringify(model.hoverDetail?.items)}`,
    );
    const departed = model.series.find(s => s.id === departedId);
    assert.equal(departed?.name, 'EarlyDepartedName', 'departed player uses dated name <= displayed hour, not future branch-latest name');
  });

  it('formats derived eAPM from exact recorded operands without float drift or Math.round and shows operands in hover tooltip', () => {
    const exactId = 'user:exact';
    const smallId = 'user:small';
    const halfId = 'user:half';
    const driftFloat = 3 / (3 / 10);
    const head = {
      available: true,
      players: [],
      series: [
        {
          id: exactId,
          name: 'Exact',
          nameMissing: null,
          tag: 'GER',
          hour: H1943,
          summary: {
            apm: driftFloat,
            confirmedApm: driftFloat,
            orders: '3',
            confirmedOrders: '3',
            pausedOrders: '0',
            unpausedMinutes: 3 / 10,
            unpausedMinutesNum: '3',
            unpausedMinutesDen: '10',
            complete: true,
            gaps: false,
          },
          points: [],
        },
        {
          id: halfId,
          name: 'Half',
          nameMissing: null,
          tag: 'ITA',
          hour: H1943,
          summary: {
            apm: 29.5,
            confirmedApm: 29.5,
            orders: '59',
            confirmedOrders: '59',
            pausedOrders: '0',
            unpausedMinutes: 2,
            unpausedMinutesNum: '2',
            unpausedMinutesDen: '1',
            complete: true,
            gaps: false,
          },
          points: [],
        },
        {
          id: smallId,
          name: 'Small',
          nameMissing: null,
          tag: 'FRA',
          hour: H1943,
          summary: {
            apm: 1 / 3,
            confirmedApm: 1 / 3,
            orders: '1',
            confirmedOrders: '1',
            pausedOrders: '0',
            unpausedMinutes: 3,
            unpausedMinutesNum: '3',
            unpausedMinutesDen: '1',
            complete: true,
            gaps: false,
          },
          points: [],
        },
      ],
      warnings: [],
    } as unknown as ApmReply;
    const tiledMap = new Map(head.series.map(s => [s.id, boundTiledSeries(makeTiled(s.id, s.summary.apm!), displayedHour(H1943))]));
    const readView = createBoundedReadView({
      hour: H1943,
      panels: new Map([['head', head]]),
      series: new Map([['apm', tiledMap]]),
      hovers: new Map([['hover', { hour: H1943, data: head }]]),
    });
    const model = apmGraphProjection.project(readView, {
      group: 'player',
      extent: [H1936, H1943],
      surface: '#111111',
      gaps: [],
      yMin: null,
      yMax: null,
      available: true,
    });
    assert.equal(model.series[0]?.endLabel?.value, '10', '3 actions in 3/10 min is exact 10, not float 9.999999999999998 -> 9');
    assert.equal(model.series[1]?.endLabel?.value, '29', '59 actions in 2 min (29.5) truncates toward zero to 29, never Math.round 30');
    assert.equal(model.series[2]?.endLabel?.value, '<1', '1 action in 3 min prints <1, never 0');
    assert.equal(model.auditRows?.find(r => r.row === exactId)?.value, '10');
    assert.equal(model.auditRows?.find(r => r.row === halfId)?.value, '29');
    assert.equal(model.auditRows?.find(r => r.row === smallId)?.value, '<1');
    const items = model.hoverDetail?.items ?? [];
    assert.ok(items.some(item => item.label === 'Exact' && item.value === '10'), 'hover rate uses exact operands');
    assert.ok(items.some(item => item.label === 'Half' && item.value === '29'), 'hover rate truncates toward zero');
    assert.ok(items.some(item => item.label === 'Small' && item.value === '<1'), 'hover sub-unit rate prints <1');
    assert.ok(items.some(item => item.label.includes('3 actions') && item.label.includes('0.3 min')), `hover shows recorded operands for Exact: ${JSON.stringify(items)}`);
    assert.ok(items.some(item => item.label.includes('1 action') && item.label.includes('3 min')), `hover shows recorded operands for Small: ${JSON.stringify(items)}`);
  });

  it('R2-S18: reads exact single-hour eAPM for head, end label, and audit through readPlanExact and omits heads for departed players', async () => {
    const h0 = H1936;
    const h1 = H1936 + 1;
    const plan = apmGraphDeclaration.sources({
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1', firstHour: h0, lastHour: h1, gaps: [] },
      countries: { all: true, tags: [], top: null },
      options: { group: 'player' },
      aux: {
        available: true,
        warnings: [],
        players: [
          { id: 'user:active', name: 'Active', nameMissing: null, tags: ['GER'], runs: [{ firstHour: h0, lastHour: h1, tag: 'GER', name: 'Active', nameMissing: null }] },
          { id: 'user:departed', name: 'Departed', nameMissing: null, tags: ['ITA'], runs: [{ firstHour: h0, lastHour: h0, tag: 'ITA', name: 'Departed', nameMissing: null }] },
        ],
        series: [],
      },
      gateSpan: (a: number, b: number) => ({ firstHour: a, lastHour: b }),
    } as never, { width: 800, height: 300, following: true, staticSpan: null });

    const headEntry = plan.entries.find(e => e.name === 'head')!;
    assert.equal(headEntry.read.shape, 'point', 'apm-graph head must be a single-hour point read, not a 168-hour trailing window');
    const followParams = (headEntry.read as { followQuery?: { params?: { windowHours?: number } } }).followQuery?.params;
    assert.equal(followParams?.windowHours, 1, 'followed head must use a 1-hour window matching the hourly line');

    const requests: { product: string; params: Record<string, unknown> }[] = [];
    const client = {
      product: async (req: { product: string; params: Record<string, unknown> }) => {
        requests.push({ product: req.product, params: req.params });
        if (req.params.hour === h1) {
          return {
            data: {
              available: true,
              warnings: [],
              players: [],
              series: [{
                id: 'user:active',
                name: 'Active',
                nameMissing: null,
                tag: 'GER',
                hour: h1,
                summary: { apm: 20, confirmedApm: 20, orders: '20', confirmedOrders: '20', pausedOrders: '0', unpausedMinutes: 1, unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: false },
                points: [],
              }],
            } satisfies ApmReply,
          };
        }
        return {
          data: {
            available: true,
            warnings: [],
            players: [],
            series: [
              {
                id: 'user:active',
                name: 'Active',
                nameMissing: null,
                tag: 'GER',
                hour: h1,
                summary: { apm: 10, confirmedApm: 10, orders: '20', confirmedOrders: '20', pausedOrders: '0', unpausedMinutes: 2, unpausedMinutesNum: '2', unpausedMinutesDen: '1', complete: true, gaps: false },
                points: [],
              },
              {
                id: 'user:departed',
                name: 'Departed',
                nameMissing: null,
                tag: 'ITA',
                hour: h0,
                summary: { apm: 15, confirmedApm: 15, orders: '15', confirmedOrders: '15', pausedOrders: '0', unpausedMinutes: 1, unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: false },
                points: [],
              },
            ],
          } satisfies ApmReply,
        };
      },
    };
    const exactView = await readPlanExact(plan, displayedHour(h1), {
      client: client as never,
      following: true,
      timeMode: 'follow',
      branch: { firstHour: h0, lastHour: h1 },
      window: { firstHour: h0, lastHour: h1 },
      staticSpan: null,
      gate: freezeGate({
        recorded: h => h as never,
        span: (a, b) => ({ firstHour: a, lastHour: b }) as never,
      }),
    });
    assert.ok(exactView);
    assert.equal(requests.length, 1);
    assert.equal(requests[0]?.params.hour, h1, 'readPlanExact must request exact hour h1, not a trailing span');

    const model = apmGraphProjection.project(exactView, {
      group: 'player',
      extent: [h0, h1],
      surface: '#111111',
      gaps: [],
      yMin: null,
      yMax: null,
      available: true,
    });
    assert.equal(model.series[0]?.headValue, 20);
    assert.equal(model.series[0]?.headHour, h1);
    assert.equal(model.series[0]?.endLabel?.value, '20');
    assert.equal(model.auditRows?.find(r => r.row === 'user:active')?.value, '20');

    const staleDepartedView = createBoundedReadView({
      hour: h1,
      panels: new Map([['head', {
        available: true,
        warnings: [],
        players: [],
        series: [{
          id: 'user:departed',
          name: 'Departed',
          nameMissing: null,
          tag: 'ITA',
          hour: h0,
          summary: { apm: 15, confirmedApm: 15, orders: '15', confirmedOrders: '15', pausedOrders: '0', unpausedMinutes: 1, unpausedMinutesNum: '1', unpausedMinutesDen: '1', complete: true, gaps: false },
          points: [],
        }],
      } satisfies ApmReply]]),
    });
    const staleModel = apmGraphProjection.project(staleDepartedView, {
      group: 'player',
      extent: [h0, h1],
      surface: '#111111',
      gaps: [],
      yMin: null,
      yMax: null,
      available: true,
    });
    assert.equal(staleModel.series[0]?.headValue, null, 'departed player observed only at h0 < h1 gets no head value at h1');
    assert.equal(staleModel.series[0]?.endLabel, null, 'departed player gets no end label at h1');
    assert.equal(staleModel.auditRows?.length, 0, 'departed player is not audited as a head at h1');
  });
});
