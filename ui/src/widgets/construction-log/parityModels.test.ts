import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { LogListProps } from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ConstructionLogManifestReply, ConstructionLogReply, ConstructionProject, EventFeedReply, ShipsSunkReply } from '../../generated/dashboard';
import { filteredEventRows } from '../event-feed/eventFeedData.ts';
import type { FeedEvent } from '../event-feed/eventFeedData';
import { sinkingWindowRows } from '../ships-sunk/shipsSunkWindows.ts';
import type { SinkingItem } from '../ships-sunk/shipsSunkWindows';
import { mergeWindows } from './constructionData.ts';
import type { ProjectAt } from './constructionData';

test('parity list models complete without sweeping offscreen dated labels or ship titles', async () => {
  const interval = { firstHour: 100, lastHour: 6000 };
  const project: ConstructionProject = { id: 'project', kind: 'building', tag: 'mod country', building: 0, repair: false,
    state: 1, toState: null, province: null, routeProvinces: null, baseLevel: null, targetLevel: null, costRaw: '12345678901234567890',
    createdHour: 100, firstRead: 100, runs: [{ ...interval, factories: 7, remaining: 3, repairLevels: 0 }],
    earlierUnknown: false, built: [], unaccounted: [], possible: [], added: [], end: null, firstRepairTick: null };
  const manifest: ConstructionLogManifestReply = { costScale: 100000, windows: [interval], recorded: true, repairIcon: null, warnings: [],
    types: [{ id: 1, token: 'mod building', name: 'Synthetic building', icon: 'data:decorative-art' }] };
  const reply: ConstructionLogReply = { window: 0, interval, outputReads: [6000], outputUnavailable: [], projects: [project], outputs: [], warnings: [] };
  const ships: ShipsSunkReply = { branchId: '1', available: true, ...interval, firstRead: 100, unread: [], warnings: [],
    classes: [{ token: 99, key: 'mod ship', name: 'Synthetic ship class', icon: 'data:decorative-art' }],
    places: ['Synthetic sea'], recordedNames: ['Recorded class'], rows: Array.from({ length: 5000 }, (_, index) => ({
      sinkHour: index + 100, knownAt: index + 100, name: `Ship ${index}`, classIndex: 0, recordedNameIndex: 0,
      victim: `victim ${index}`, attacker: `attacker ${index}`, victimAsOf: null, attackerAsOf: null, placeIndex: 0,
      cause: 'hostile', attributionName: null, assistedKill: false,
    })) };
  const eventReply: EventFeedReply = { branchId: '1', ...interval, windows: [], rows: [{ id: 'event', kind: 'focusCompleted', category: 'milestones',
    hour: 120, readHour: 140, nameHour: 120, exact: false, tags: ['mod country'], item: 0, amount: null },
    { id: 'offscreen-event', kind: 'focusCompleted', category: 'milestones', hour: 110, readHour: 130,
      nameHour: 110, exact: false, tags: ['offscreen country'], item: 0, amount: null }],
    items: [{ token: 'mod focus', name: '', description: 'Recorded description', icon: 'data:decorative-art', idea: null }],
    modifiers: [], texts: [], unavailable: [], warnings: [] };
  const eventRows = filteredEventRows(eventReply, { categories: new Set(['milestones']), country: () => true, firstHour: null });
  const sinkingWindows = new Map([[0, { window: 0, indices: ships.rows.map((_, index) => index), reply: ships }]]);
  const state = { construction: { manifest, log: mergeWindows(manifest, new Map([[0, reply]]), 0), hour: 6000 },
    shipsManifest: { windows: [interval], classes: ships }, sinkingWindows, windowRequests: 0,
    feed: { data: eventReply, identity: 'feed', pages: [{ key: '0', start: 100, end: 6000, count: eventRows.length, rows: eventRows }], warnings: [], errors: new Map() },
    visible: [] as readonly LogRow<unknown>[], log: undefined as LogListProps<unknown> | undefined,
    titleIndices: [] as readonly number[], wantedCountries: [] as unknown, wantedStates: [] as unknown, preparePages: false };
  const host = globalThis as typeof globalThis & { __cheapParityModels?: typeof state };
  host.__cheapParityModels = state;
  const replacements: Record<string, string> = {
    '/widgets/construction-log/useConstructionLog.ts': `export const useConstructionLog = () => ({ ...globalThis.__cheapParityModels.construction,
      retry() {}, onDrawn() {} });`,
    '/widgets/ships-sunk/useShipsSunk.ts': `export const useShipsSunk = (_widget, runtime) => ({ track: runtime.tracks[0], data: globalThis.__cheapParityModels.shipsManifest, retry() {} });
      export const useShipsSunkBattleTitles = (_runtime, _track, rows) => { globalThis.__cheapParityModels.titleIndices = rows;
        return { data: { rows: rows.map(index => ({index, hour: 100, name: 'Synthetic battle', missing: null})), warnings: [] } }; };`,
    '/dashboard/productWindows.ts': `export const useProductWindows = (_client, _identity, request, wanted) => {
      const f = globalThis.__cheapParityModels;
      if (++f.windowRequests > 1) throw new Error('Repeated ship window request');
      if (wanted.length !== 1 || wanted[0] !== 0 || request(0).product !== 'ships-sunk') throw new Error('Unexpected ship window');
      return { data: f.sinkingWindows, errors: new Map(), retry() {} };
    };`,
    '/widgets/event-feed/useEventFeed.ts': `export const useEventFeed = (_runtime, _track, _active, _hour, _filter, _filterKey, prepareAll = false) => {
      globalThis.__cheapParityModels.preparePages = prepareAll;
      return { ...globalThis.__cheapParityModels.feed, onWantedPages() {}, retry() {}, retryPages() {} }; };`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => {
      globalThis.__cheapParityModels.wantedCountries = wanted;
      return { labelAt: hour => tag => ({ name: tag + ' at ' + hour, known: true }) }; };`,
    '/dashboard/useStateNamesAt.ts': `export const useStateNamesAt = (_client, _track, wanted) => {
      globalThis.__cheapParityModels.wantedStates = wanted;
      return (hour, state) => ({state, name: 'State ' + state + ' at ' + hour}); };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 6000;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({retry() {}});',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({categories: ['#112233', '#223344', '#334455', '#445566', '#556677', '#667788']});`,
    '/charts/log/LogList.tsx': `import {createElement} from 'react'; export const PreparedLogCoverage = () => null; export default props => {
      globalThis.__cheapParityModels.log = props;
      const row = props.rows?.[0] ?? props.pages?.[0]?.rows?.[0];
      return row ? createElement('div', {}, props.renderRow(row)) : null; };`,
  };
  const vite = await createServer({ configFile: false, mode: 'parity', optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'cheap-parity-models', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/dashboard/parityPreparation.ts')) {
          assert.ok(source.includes('return { requested, prepare };'));
          return source.replace('return { requested, prepare };', 'return { requested: true, prepare };');
        }
        if (['ConstructionLogWidget.tsx', 'ShipsSunkWidget.tsx', 'EventFeedWidget.tsx'].some(name => path.endsWith(`/${name}`)))
          return source.replace('useMemo, useState', 'useMemo').replace("from 'react';", "from 'react'; import {useState} from 'cheap-parity-visible';");
      },
      resolveId(id) { if (id === 'cheap-parity-visible') return '\0cheap-parity-visible'; },
      load(id) { if (id === '\0cheap-parity-visible') return 'export const useState = () => [globalThis.__cheapParityModels.visible, () => {}];'; },
    }] });
  const props = { widget: { trackIds: ['main'], countries: { kind: 'all' }, options: {}, time: { mode: 'campaign' } }, active: true,
    runtime: { client: {}, clock: { getHour: () => 6000 }, tracks: [{ id: 'main', source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign' } },
      branch: { id: '1', ...interval } }] }, onDiagnostics() {} } as unknown as WidgetProps;
  try {
    const load = async (path: string) => (await vite.ssrLoadModule(path)).default;
    const Construction = await load('/ui/src/widgets/construction-log/ConstructionLogWidget.tsx');
    renderToStaticMarkup(createElement(Construction, props));
    let construction = state.log as LogListProps<ProjectAt>;
    assert.equal(construction.paritySemantic!.complete, true);
    const projectRow = construction.rows![0];
    assert.equal(projectRow.item.project.costRaw, '12345678901234567890');
    const cells = construction.paritySemantic!.content!(projectRow) as Record<string, unknown>;
    assert.deepEqual(cells, { id: 1, token: 'mod building', name: 'Synthetic building' });
    assert.doesNotMatch(JSON.stringify(cells), /decorative-art/);
    state.visible = [projectRow];
    const markup = renderToStaticMarkup(createElement(Construction, props));
    construction = state.log as LogListProps<ProjectAt>;
    assert.match(markup, /mod country at 100/);
    assert.match(markup, /State 1 at 6000/);
    assert.deepEqual(state.wantedStates, [{ hour: 6000, state: 1 }]);
    state.construction = { ...state.construction, manifest: { ...manifest, windows: [{ firstHour: 0, lastHour: 99 }, interval] },
      log: mergeWindows({ ...manifest, windows: [{ firstHour: 0, lastHour: 99 }, interval] }, new Map([[1, { ...reply, window: 1, projects: [] }]]), 1) };
    state.visible = [];
    renderToStaticMarkup(createElement(Construction, props));
    assert.equal(state.log!.paritySemantic!.complete, false, 'an empty newest window cannot certify complete construction history');

    const Ships = await load('/ui/src/widgets/ships-sunk/ShipsSunkWidget.tsx');
    const shipRows = sinkingWindowRows(sinkingWindows, 6000, [], 'both', () => true, null, 'campaign');
    state.visible = [shipRows[0]];
    renderToStaticMarkup(createElement(Ships, props));
    const shipList = state.log as LogListProps<SinkingItem>;
    assert.equal(state.windowRequests, 1, 'the 5,000-row model requests its prepared window once');
    assert.equal(shipList.rows!.length, 5000);
    assert.equal(shipList.paritySemantic!.complete, true);
    assert.deepEqual(state.titleIndices, [4999], 'full model preparation must retain ordinary drawn-only title demand');
    assert.equal((state.wantedCountries as unknown[]).length, 2);
    const shipCells = shipList.paritySemantic!.content!(shipList.rows!.at(-1)!) as Record<string, unknown>;
    assert.deepEqual([(shipCells.class as {name: string}).name, shipCells.recordedName, shipCells.place, shipCells.battleTitleIndex], ['Synthetic ship class', 'Recorded class', 'Synthetic sea', 0]);
    assert.doesNotMatch(JSON.stringify(shipCells), /decorative-art/);

    const Event = await load('/ui/src/widgets/event-feed/EventFeedWidget.tsx');
    state.visible = [eventRows[0]];
    const eventMarkup = renderToStaticMarkup(createElement(Event, props));
    const eventList = state.log as LogListProps<FeedEvent>;
    assert.equal(state.preparePages, true);
    assert.equal(eventList.paritySemantic!.complete, true);
    assert.equal(eventList.pages!.flatMap(page => page.rows ?? []).length, 2, 'complete preparation includes offscreen event operands');
    assert.deepEqual(state.wantedCountries, [{ hour: 120, tag: 'mod country' }], 'preparing offscreen events must not request their dated labels');
    const eventModel = eventList.paritySemantic!.item!(eventRows[0]) as { event: { readHour: number }; item: { name: string } };
    assert.equal(eventModel.event.readHour, 140, 'the raw model retains the actual supplying-read cutoff');
    assert.equal(eventModel.item.name, '', 'localized empty titles remain exact');
    assert.doesNotMatch(JSON.stringify(eventModel), /decorative-art|\"reply\"/);
    assert.match(eventMarkup, /mod country at 120/);
    assert.equal(eventList.describe(eventRows[0]), 'mod country at 120 completed focus ');
  } finally { await vite.close(); delete host.__cheapParityModels; }
});
