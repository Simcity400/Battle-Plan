import { pictureError } from '../../shell/perf.ts';
import { auditCell, auditRow, type FamilyAuditRow } from '../auditRow.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { tooltipStore } from '../../charts/tooltip.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import type { Pipeline, PointerHitMeta } from '../../dashboard/follow/pipeline.ts';
import { AuditSnapshotStore } from '../audit.ts';
import { barsHostAdapter, parseBarsHit } from '../bars/hostAdapter.ts';
import { createBoundedReadView, displayedHour } from '../boundedView.ts';
import type { FamilyTheme, InstalledAction, InstalledPicture, MirrorModel } from '../contract.ts';
import { heatLanesHostAdapter, heatMatrixHostAdapter, parseHeatLanesHit, parseHeatMatrixHit } from '../heat/hostAdapter.ts';
import { parseTimeChartHit, timeChartHostAdapter } from '../time-chart/hostAdapter.ts';
import { familyBinding, lazyActionKeys, nextReportedRowWindow, reconcileInstalledRowWindow, requestedRowWindow, type FamilyBindingContext, type FamilyHostAdapter, type ParsedFamilyHit } from './adapter.ts';
import { BoundSelections, displayedHoverKeys, installedRecordOf, recordInstalled, resolveHostInput, type HostInputGate } from './installed.ts';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { hoverAsks, readPlanExact } from '../broker.ts';
import type { SourcePlan } from '../contract.ts';
import type { FrozenGate } from '../sources.ts';
import { parseTableHit, tableHostAdapter } from '../table/hostAdapter.ts';
import { cardHostAdapter } from '../card/hostAdapter.ts';
import { pagedTailEntry } from '../table/logHelpers.ts';
import { DEFAULT_TAIL_PAGE_SIZE, SnapshotPager } from '../table/rowSource.ts';
import { ROW_HEIGHT, tableSettingsOf } from '../table/settings.ts';
import { rowExtents } from '../table/layout.ts';
import type { TableModel } from '../table/model.ts';
import { createBoundAuditReader, createPlanAuditVerifier, type BoundAuditEntry, type PlanAuditSnapshot } from './auditBinding.ts';
import { boundDated } from './dated.ts';
import { NO_EXPANSION, toggleExpansion } from './expansion.ts';
import {
  handleFamilyGridKeyDown,
  handleFamilyKeyDown,
  handleFamilyPointsKeyDown,
  navPageSteps,
  nearestDataOrdinal,
  resolveInstalledOrdinal,
  tableOrdinalBounds,
  tablePageSteps,
  usableRowHeight,
  type FamilyNavTuple,
} from './keyboard.ts';
import { keepEqual } from './stableValue.ts';
import { rowHoverPublisher } from './hoverRow.ts';
import { mirrorToSync, optionElementId, reconcileFocusedKey, syncListboxMirror, syncTableMirror } from './mirror.ts';
import { bindFamilyPointer, CLICK_CANCEL_PX, parseDefaultRowHit } from './pointer.ts';
import { computeScrollGeometry, logicalToPhysicalTop, physicalToLogicalTop } from './scroll.ts';
import { reportFamilyShownHour } from './shownHour.ts';
import { reducedMotionEnabled, sameFamilyTheme } from './theme.ts';
import {
  buildTooltipContent,
  FamilyTooltipController,
  HOVER_CLOSE_MS,
  resolveTooltipEntryContent,
  type FamilyActionPayload,
  type FamilyTooltipEntry,
} from './tooltips.ts';
import { centerWindowPosition, clampWindowPosition } from './windowPlacement.ts';
import { gatedByCountry, planFamilySources } from './planSources.ts';
import { factionPooledResourcesDeclaration } from '../../widgets/faction-pooled-strength/factionPooledStrengthProjection.ts';
import { identitySnapshots } from './identitySnapshots.ts';
import { countryMatch, EVER_UNRECORDED, everBounds, everStateAt, groupMembers, groupRuns, groupUnobserved, membershipUnrecordedAt, rankedCandidates, resolveCountries, tagsAt, worldFilter } from '../../dashboard/groups.ts';
import { NO_MEMBERS } from '../../dashboard/noMembers.ts';
import { effectiveCountryUnobserved } from '../sources.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { IdentityBroker, type IdentityTransport } from '../identity/broker.ts';
import { flagChoiceKey, type IdentityScope } from '../identity/types.ts';
import { flagArtHandle, requestDemandedFlags } from '../art/provider.ts';
import { mergeAccessibleDescription, mergeDiagnosticNotes, resolveHeaderSummary } from './headerSummary.ts';
import { airKillsLossesLinesDeclaration } from '../../widgets/air-kills-losses/airKillsLossesLinesProjection.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { countryKey } from '../../dashboard/countryKey.ts';

test('hover requests the installed ranking and projection slots while older tiles are held', () => {
  const source = { id: 'source', revision: '1' } as DashboardSource;
  const plan: SourcePlan = { owner: 'rank', entries: [
    { name: 'hover', source, branch: '1', offset: 0, read: { shape: 'exact-hover', product: 'series-state', params: {}, readsOf: 'main' } },
    { name: 'detail', source, branch: '1', offset: 0, read: { shape: 'exact-hover', product: 'series-rank', params: {}, countriesOf: 'bands' } },
  ] };
  const tiled = { main: new Map([['OLD', 1]]), bands: new Map([['OLD', 2]]), named: new Map([['OLD', 3]]) };
  const keys = { main: ['NEW'], bands: [countryKey(9)] };
  const ask = (drawn?: typeof keys) => hoverAsks(plan, 15, name => displayedHoverKeys(name, drawn, tiled), undefined, drawn).map(item => item.request.params);
  assert.deepEqual(ask(keys), [{ reads: [{ country: 'NEW', hour: 15 }] }, { countries: [9], hour: 15 }]);
  assert.deepEqual(ask({ ...keys, bands: [] }), [{ reads: [{ country: 'NEW', hour: 15 }] }]);
  const world: SourcePlan = { owner: 'world', entries: [
    { name: 'hover', source, branch: '1', offset: 0, read: { shape: 'exact-hover', product: 'world-tension', params: {}, countriesOf: 'bands', allowEmptyCountries: true } },
  ] };
  assert.deepEqual(hoverAsks(world, 15, () => []).map(item => item.request.params), [{ countries: [], hour: 15 }]);
  assert.deepEqual(ask()[0], { reads: [{ country: 'OLD', hour: 15 }] });
  const modelsPlan: SourcePlan = { owner: 'production', entries: [{
    name: 'hover', source, branch: '1', offset: 0, read: {
      shape: 'exact-hover', product: 'equipment-produced-race', params: {}, countriesOf: 'main', countryParamsOf: { models: 'named' },
    },
  }] };
  const models = (drawn: Record<string, string[]>) => hoverAsks(modelsPlan, 15, name => displayedHoverKeys(name, drawn, tiled), undefined, drawn)[0]?.request.params;
  assert.deepEqual(models(keys), { countries: ['NEW'], hour: 15 });
  assert.deepEqual(models({ ...keys, named: [] }), { countries: ['NEW'], hour: 15, models: [] });
  assert.deepEqual(models({ main: ['AAA', countryKey(9)], named: [countryKey(9)] }), { countries: ['AAA', 9], hour: 15, models: [9] });
});

const mkEvent = (key: string, extra: Partial<KeyboardEvent> = {}) => {
  let prevented = false;
  const event = {
    key,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...extra,
    preventDefault() { prevented = true; },
  } as unknown as KeyboardEvent;
  return { event, prevented: () => prevented };
};

test('compressed scroll geometry preserves linear range and maps large extents accurately', () => {
  const linear = computeScrollGeometry(2400, 400, 8_000_000);
  assert.equal(linear.compressed, false);
  assert.equal(linear.physicalHeight, 2400);
  assert.equal(physicalToLogicalTop(600, linear), 600);
  assert.equal(logicalToPhysicalTop(600, linear), 600);

  const huge = computeScrollGeometry(40_000_000, 500, 8_000_000);
  assert.equal(huge.compressed, true);
  assert.equal(huge.physicalHeight, 8_000_000);
  assert.equal(physicalToLogicalTop(0, huge), 0);
  assert.equal(physicalToLogicalTop(huge.physicalHeight - huge.viewportHeight, huge), huge.logicalHeight - huge.viewportHeight);
  const midPhysical = (huge.physicalHeight - huge.viewportHeight) / 2;
  const midLogical = physicalToLogicalTop(midPhysical, huge);
  assert.ok(Math.abs(midLogical - (huge.logicalHeight - huge.viewportHeight) / 2) < 1);
  assert.ok(Math.abs(logicalToPhysicalTop(midLogical, huge) - midPhysical) < 1);
});

test('reconcileFocusedKey respects fallbackToFirst flag', () => {
  assert.deepEqual(reconcileFocusedKey('b', ['a', 'b', 'c'], false), { key: 'b', present: true, index: 1 });
  assert.deepEqual(reconcileFocusedKey('evicted', ['a', 'b'], false), { key: null, present: false, index: -1 });
  assert.deepEqual(reconcileFocusedKey('evicted', ['a', 'b'], true), { key: 'a', present: false, index: 0 });
});

test('F7 & F10: paged keyboard sends first ArrowDown/ArrowUp from no focus to ordinal 0, and empty Escape returns early', () => {
  const ordinals: number[] = [];
  const down0 = mkEvent('ArrowDown');
  handleFamilyKeyDown(down0.event, {
    nav: [],
    totalCount: 500,
    focusedKey: null,
    pageStep: { down: 10, up: 10 },
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: () => {},
    onFocusOrdinal: ord => ordinals.push(ord),
    onActivate: () => {},
  });
  assert.equal(down0.prevented(), true);

  const up0 = mkEvent('ArrowUp');
  handleFamilyKeyDown(up0.event, {
    nav: [],
    totalCount: 500,
    focusedKey: null,
    pageStep: { down: 10, up: 10 },
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: () => {},
    onFocusOrdinal: ord => ordinals.push(ord),
    onActivate: () => {},
  });
  assert.equal(up0.prevented(), true);

  const end = mkEvent('End');
  handleFamilyKeyDown(end.event, {
    nav: [],
    totalCount: 500,
    focusedKey: null,
    pageStep: { down: 10, up: 10 },
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: () => {},
    onFocusOrdinal: ord => ordinals.push(ord),
    onActivate: () => {},
  });
  assert.deepEqual(ordinals, [0, 0, 499]);

  let unpinned = false;
  const esc = mkEvent('Escape');
  handleFamilyKeyDown(esc.event, {
    nav: [],
    focusedKey: null,
    pageStep: { down: 10, up: 10 },
    blocked: false,
    isPinnedTooltip: () => true,
    onFocusKey: () => {},
    onActivate: () => {},
    onUnpinTooltip: () => { unpinned = true; },
  });
  assert.equal(esc.prevented(), false, 'F10: Escape with zero rows returns early without preventDefault');
  assert.equal(unpinned, false, 'F10: Escape with zero rows does not unpin');
});

test('handleFamilyKeyDown avoids activating row 0 when focus key was evicted and supports typeahead', () => {
  const nav: FamilyNavTuple[] = [
    ['GER', 'Germany', 0, -1, '', null],
    ['FRA', 'France', 1, -1, '', null],
    ['ITA', 'Italy', 2, -1, '', null],
  ];
  const activated: string[] = [];
  const focused: string[] = [];
  const typeaheadRef = { current: { query: '', until: 0 } };

  const enter = mkEvent('Enter');
  handleFamilyKeyDown(enter.event, {
    nav,
    focusedKey: 'evicted-key',
    pageStep: { down: 5, up: 5 },
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: key => focused.push(key),
    onActivate: entry => activated.push(entry[0]),
  });
  assert.equal(enter.prevented(), true);
  assert.deepEqual(activated, [], 'Enter on an evicted row activates nothing, never the first remaining row');
  const tree: FamilyNavTuple[] = [['GRP', 'Group', 0, 0, '', null], ['GER', 'Germany', 1, -1, 'GRP', null]];
  const toggled: string[] = [];
  for (const key of ['ArrowRight', 'ArrowLeft']) {
    handleFamilyKeyDown(mkEvent(key).event, { nav: tree, focusedKey: 'evicted-key', pageStep: { down: 5, up: 5 }, blocked: false, isPinnedTooltip: () => false, onFocusKey: k => focused.push(k), onActivate: () => {}, onToggleTree: k => toggled.push(k) });
  }
  assert.deepEqual([toggled, focused], [[], []], 'tree keys on an evicted row expand or move nothing');

  const keyOptions = (focusedKey: string, blocked: boolean) => ({
    nav,
    focusedKey,
    pageStep: { down: 5, up: 5 },
    blocked,
    isPinnedTooltip: () => false,
    onFocusKey: (key: string) => (focused as string[]).push(key),
    onActivate: (entry: FamilyNavTuple) => (activated as string[]).push(entry[0]),
  });
  handleFamilyKeyDown(mkEvent('Enter').event, keyOptions('FRA', true));
  assert.deepEqual(activated, [], 'a blocked host activates nothing');
  handleFamilyKeyDown(mkEvent(' ').event, keyOptions('FRA', false));
  assert.deepEqual(activated, ['FRA'], 'Space activates the focused row');
  activated.length = 0;

  const search = mkEvent('i');
  handleFamilyKeyDown(search.event, {
    nav,
    focusedKey: 'GER',
    pageStep: { down: 5, up: 5 },
    blocked: false,
    typeaheadRef,
    isPinnedTooltip: () => false,
    onFocusKey: key => focused.push(key),
    onActivate: () => {},
  });
  assert.equal(search.prevented(), true);
  assert.deepEqual(focused, ['ITA']);
});

test('F3: sameFamilyTheme compares drawn tokens by value and rejects changed tokens', () => {
  const base: FamilyTheme = {
    surface: '#111',
    text: '#eee',
    muted: '#888',
    line: '#333',
    lineSoft: '#222',
    accent: '#38f',
    blue: '#38f',
    red: '#f44',
    amber: '#fa0',
    green: '#2c5',
    grey: '#888',
    hatch: '#555',
    hatchEdge: '#444',
    font: 'Inter',
    fontSize: { small: 11, body: 13 },
    lineWidth: 1,
    hatchSpacing: 6,
  };
  assert.equal(sameFamilyTheme(base, { ...base, fontSize: { small: 11, body: 13 } }), true);
  assert.equal(sameFamilyTheme(base, { ...base, surface: '#000' }), false);
  assert.equal(sameFamilyTheme(base, { ...base, fontSize: { small: 12, body: 13 } }), false);
});

test('F4: reducedMotionEnabled honors app dataset.motion and OS matchMedia independently', () => {
  const prevDoc = globalThis.document;
  const prevMatch = globalThis.matchMedia;
  try {
    let osMatches = false;
    const dataset: Record<string, string | undefined> = {};
    (globalThis as { document: unknown }).document = { documentElement: { dataset } };
    (globalThis as { matchMedia: unknown }).matchMedia = () => ({ matches: osMatches });

    assert.equal(reducedMotionEnabled(), false, 'both off -> false');
    dataset.motion = 'reduced';
    osMatches = false;
    assert.equal(reducedMotionEnabled(), true, 'app setting on, OS off -> true');
    dataset.motion = undefined;
    osMatches = true;
    assert.equal(reducedMotionEnabled(), true, 'app setting off, OS on -> true');
  } finally {
    (globalThis as { document: unknown }).document = prevDoc;
    (globalThis as { matchMedia: unknown }).matchMedia = prevMatch;
  }
});

test('F6: bound audit reader verifies an earlier snapshot with the verifier captured at creation time', async () => {
  const store = new AuditSnapshotStore<BoundAuditEntry<{ readonly tag: string }>>(16);
  const reader = createBoundAuditReader(store);
  const calls: string[] = [];
  const idA = store.create({
    snapshot: { tag: 'snapA' },
    verify: async (hour, snap) => {
      calls.push(`verifierA:${snap.tag}@${hour}`);
      return { values: new Map([['row', 'A']]) };
    },
  });
  const idB = store.create({
    snapshot: { tag: 'snapB' },
    verify: async (hour, snap) => {
      calls.push(`verifierB:${snap.tag}@${hour}`);
      return { values: new Map([['row', 'B']]) };
    },
  });
  const resA = await reader(100, idA);
  const resB = await reader(200, idB);
  assert.deepEqual(calls, ['verifierA:snapA@100', 'verifierB:snapB@200']);
  assert.equal(resA?.values.get('row'), 'A');
  assert.equal(resB?.values.get('row'), 'B');

  const shown = [auditCell('GER', 'label', 'Germany'), auditCell('GER', 'value', '3'), auditCell('FRA', 'label', 'France'), auditCell('FRA', 'value', '2')];
  const exactOf = (rows: readonly FamilyAuditRow[]) => createPlanAuditVerifier<null, Record<string, never>>()(5, {
    projection: { family: 'bars', kind: 'synthetic-audit', project: () => null },
    options: {},
    exactReadView: async () => createBoundedReadView({ hour: 5, panels: new Map() }),
    deriveRows: () => rows,
  }, undefined);
  assert.equal(pictureError('bars', shown, await exactOf(shown)), null, 'two rows with the same cell names compare cell by cell and a correct picture passes');
  await assert.rejects(exactOf([auditRow('GER', '3'), auditRow('GER', '4')]), /Audit key GER repeats/, 'a repeated audit key is refused, never collapsed');
  const ranged = new AuditSnapshotStore<BoundAuditEntry<PlanAuditSnapshot<readonly string[], Record<string, never>>>>(16);
  const rangedId = ranged.create({ verify: createPlanAuditVerifier<readonly string[], Record<string, never>>(), snapshot: {
    projection: { family: 'table', kind: 'synthetic-audit', project: () => ['GER', 'FRA', 'ITA', 'ENG'] },
    options: {},
    exactReadView: async () => createBoundedReadView({ hour: 5, panels: new Map() }),
    deriveRows: (rows, _hour, fields) => {
      const range = fields?.range as { first: number; count: number } | undefined;
      return rows.slice(range?.first ?? 0, (range?.first ?? 0) + (range?.count ?? rows.length)).map(key => auditRow(key, key));
    },
  } });
  const sampled = createBoundAuditReader(ranged);
  const full = await sampled(5, rangedId, { range: { first: 0, count: 4 } });
  const shorter = await sampled(5, rangedId, { range: { first: 0, count: 2 } });
  assert.deepEqual([...full!.values.keys()], ['GER', 'FRA', 'ITA', 'ENG'], 'R4-S6: a full viewport sample is checked against its own installed range');
  assert.deepEqual([...shorter!.values.keys()], ['GER', 'FRA'], 'R4-S6: a later, shorter sample under the same audit id is checked against its own range, and the earlier one is not changed by it');
});

test('F8 & F9: listbox mirror uses stable key-based IDs, invalidates on prefix change, and honors explicit selected state', () => {
  const prevDoc = globalThis.document;
  try {
    interface MockEl {
      id: string;
      textContent: string;
      dataset: Record<string, string>;
      attrs: Map<string, string>;
      children: MockEl[];
      setAttribute(name: string, value: string): void;
      removeAttribute(name: string): void;
      replaceChildren(...nodes: MockEl[]): void;
      append(...nodes: MockEl[]): void;
    }
    const makeEl = (): MockEl => ({
      id: '',
      textContent: '',
      dataset: {},
      attrs: new Map(),
      children: [],
      setAttribute(name, value) { this.attrs.set(name, value); },
      removeAttribute(name) { this.attrs.delete(name); },
      replaceChildren(...nodes) {
        this.children = [];
        for (const n of nodes) {
          if (n.children.length > 0 && !n.attrs.has('role')) this.children.push(...n.children);
          else this.children.push(n);
        }
      },
      append(...nodes) { this.children.push(...nodes); },
    });
    (globalThis as { document: unknown }).document = {
      createElement: () => makeEl(),
      createDocumentFragment: () => makeEl(),
    };

    const listbox = makeEl();
    const lastMirrorKeyRef = { current: '' };
    const items = [
      { key: 'row:1', spoken: 'Alpha', index: 1, selected: true },
      { key: 'row_1', spoken: 'Beta', selected: false },
      { key: 'row:2', spoken: 'Gamma' },
    ];
    const first = syncListboxMirror(listbox as unknown as HTMLElement, items, {
      optionIdPrefix: 'p1',
      total: 3,
      focusedKey: 'row_1',
      lastMirrorKeyRef,
    });
    assert.equal(listbox.children.length, 3);
    const ids = listbox.children.map(el => el.id);
    assert.equal(new Set(ids).size, 3, 'F8: mixed indexed/unindexed and punctuation-similar keys produce unique IDs');
    assert.equal(first.activeDescendantId, optionElementId('p1', 'row_1'));
    assert.equal(listbox.attrs.get('aria-activedescendant'), optionElementId('p1', 'row_1'));
    assert.equal(listbox.children[0]!.attrs.get('aria-selected'), 'true', 'F9: explicit selected:true honored');
    assert.equal(listbox.children[1]!.attrs.get('aria-selected'), 'false', 'F9: explicit selected:false honored even when focused');
    assert.equal(listbox.children[2]!.attrs.get('aria-selected'), 'false', 'F9: unspecified selected defaults to false when unfocused');

    const second = syncListboxMirror(listbox as unknown as HTMLElement, items, {
      optionIdPrefix: 'p2',
      total: 3,
      focusedKey: 'row:2',
      lastMirrorKeyRef,
    });
    assert.equal(second.activeDescendantId, optionElementId('p2', 'row:2'));
    assert.equal(listbox.children[2]!.id, optionElementId('p2', 'row:2'), 'F8: prefix change rebuilds DOM option IDs');
    assert.equal(listbox.children[2]!.attrs.get('aria-selected'), 'true', 'F9: unspecified selected defaults to true when focused');
  } finally {
    (globalThis as { document: unknown }).document = prevDoc;
  }
});

test('parseDefaultRowHit distinguishes row and toggle hits, naming the subject and the inspect token an adapter without its own parser needs', () => {
  assert.deepEqual(parseDefaultRowHit('row:GER'), { key: 'GER', subject: 'GER', toggle: false, kind: 'row', inspectToken: 'row:GER' });
  assert.deepEqual(parseDefaultRowHit('toggle:grp-1'), { key: 'grp-1', subject: 'grp-1', toggle: true, kind: 'toggle', inspectToken: 'row:grp-1' });
  assert.equal(parseDefaultRowHit('header:col'), null);
  assert.equal(parseDefaultRowHit(null), null);
});

test('toggleExpansion flips the installed tree state of a row and ignores rows that are not trees', () => {
  const nav: FamilyNavTuple[] = [['war:1', 'War one', 0, 1, ''], ['war:2', 'War two', 1, 0, ''], ['war:2:GER', 'German Reich', 2, -1, 'war:2']];
  const closed = toggleExpansion(NO_EXPANSION, nav, 'war:1');
  assert.deepEqual(closed, { next: { 'war:1': false }, open: false, label: 'War one' });
  const opened = toggleExpansion(closed!.next, nav, 'war:2');
  assert.deepEqual(opened?.next, { 'war:1': false, 'war:2': true });
  assert.equal(toggleExpansion(NO_EXPANSION, nav, 'war:2:GER'), null);
  assert.equal(toggleExpansion(NO_EXPANSION, nav, 'missing'), null);
  assert.deepEqual(toggleExpansion(NO_EXPANSION, nav, 'war:2', true)?.next, { 'war:1': false, 'war:2': true }, 'exclusive: opening one closes the one that was open');
  assert.deepEqual(toggleExpansion(NO_EXPANSION, nav, 'war:1', true)?.next, { 'war:1': false }, 'closing touches nothing else');
});

test('reportFamilyShownHour prefers explicit installedHour over view.through', () => {
  const reported: (number | null)[] = [];
  const sink = (hour: number | null) => { reported.push(hour); };
  assert.equal(reportFamilyShownHour(sink, true, { mode: 'picture', target: 100, through: 90, painted: 88, dim: false, tooltip: null }, 75), 75);
  assert.equal(reportFamilyShownHour(sink, true, { mode: 'picture', target: 100, through: 90, painted: 88, dim: false, tooltip: null }), 88);
  assert.equal(reportFamilyShownHour(sink, false, { mode: 'reading', target: 100, dim: false, tooltip: null }), null);
  assert.deepEqual(reported, [75, 90, null]);
});

test('tooltip payloads carry full truncated text, exact hours, notes, missing reasons, and structured lists', () => {
  const hour = engineHour({ year: 1941, month: 9, day: 14, hour: 8 })!;
  const entry: FamilyTooltipEntry = {
    hour,
    rowKey: 'GER:battle:1',
    payload: {
      title: 'Battle of Smol…',
      fullText: 'Battle of Smolensk',
      hour,
      note: 'First seen after a recording gap.',
      exactValue: '124,850',
      missingReason: 'Air losses not recorded in this build.',
      items: [
        { label: 'Germany', value: '82,100', swatch: '#4f86f7' },
        { label: 'Soviet Union', missingReason: 'Not sampled on this hour' },
      ],
    },
  };
  const content = resolveTooltipEntryContent(entry);
  assert.equal(content.title, 'Battle of Smolensk');
  assert.equal(content.subtitle, '14 Sep 1941 · 08:00');
  assert.equal(content.note, 'First seen after a recording gap.');
  assert.equal(content.rows[0]?.value, '124,850');
  assert.equal(content.rows[1]?.valueState?.kind, 'missing');
  assert.equal(content.rows[2]?.label, 'Germany');
  assert.equal(content.rows[3]?.valueState?.kind, 'missing');

  const dated = buildTooltipContent({ title: 'Factory count', hour });
  assert.equal(dated.subtitle, '14 Sep 1941 · 08:00');

  const action: FamilyActionPayload = {
    actionId: 'open-battle',
    kind: 'battle',
    subject: 'battle:42',
    hour,
    rowKey: 'GER:battle:42',
  };
  assert.equal(action.subject, 'battle:42');
  assert.equal(action.hour, hour);
});

test('activating a row closes its tooltip, which stays closed under the opened window until the pointer or focus reaches another row', () => {
  const tooltips = new FamilyTooltipController('activated-row');
  const tip = (rowKey: string): FamilyTooltipEntry => ({ hour: 10, rowKey, content: { title: rowKey, rows: [] } });
  const sync = (inspectedKey: string | null, focusedKey: string | null = null) => tooltips.syncFromInstall({
    dim: false, held: false, inspectedKey, pointerPos: inspectedKey ? { x: 5, y: 5 } : null, hoverTip: inspectedKey ? tip(inspectedKey) : null,
    focusInside: focusedKey !== null, focusedKey, focusAnchor: { x: 0, y: 0 }, focusTip: focusedKey ? tip(focusedKey) : null, viewportHeight: 800,
  });
  sync('org');
  assert.equal(tooltips.isHoverShown(), true);
  tooltips.dismissActivated('org');
  assert.equal(tooltips.isHoverShown(), false, 'activation closes the tooltip');
  sync('org');
  assert.equal(tooltips.isHoverShown(), false, 'a later install does not reopen it over the window');
  sync(null, 'org');
  assert.equal(tooltips.isHoverShown(), false, 'the focused activated row stays quiet too');
  sync('other');
  assert.equal(tooltips.isHoverShown(), true, 'another row shows its tooltip');
  sync('org');
  assert.equal(tooltips.isHoverShown(), true, 'coming back to the row shows it again');
  tooltips.dispose();
});

test('window placement clamps and centres within viewport bounds and sameFamilyTheme detects presentation token changes', () => {
  assert.deepEqual(centerWindowPosition(400, 300, 1200, 800), { left: 400, top: 250 });
  assert.deepEqual(clampWindowPosition(-50, 999, 400, 300, 1200, 800), { left: 12, top: 488 });
  assert.deepEqual(clampWindowPosition(200, 200, 2000, 1500, 800, 600), { left: 12, top: 12 });

  const base: FamilyTheme = {
    surface: '#111',
    text: '#eee',
    muted: '#888',
    line: '#333',
    lineSoft: '#222',
    accent: '#38f',
    blue: '#38f',
    red: '#f44',
    shortage: '#ff6b6f',
    amber: '#fa0',
    green: '#2c5',
    grey: '#888',
    hatch: '#555',
    hatchEdge: '#444',
    font: 'Inter',
    fontSize: { small: 11, body: 13 },
    lineWidth: 1,
    hatchSpacing: 6,
  };
  assert.equal(sameFamilyTheme(base, { ...base, shortage: '#e5484d' }), false);
});

test('syncTableMirror updates existing table rows and cells in place without recreating nodes', () => {
  const prevDoc = (globalThis as { document?: unknown }).document;
  try {
    let createdElements = 0;
    interface MockCell {
      textContent: string;
      tabIndex: number;
      dataset: Record<string, string>;
      attrs: Map<string, string>;
      children: MockCell[];
      readonly lastElementChild: MockCell | null;
      getAttribute(name: string): string | null;
      hasAttribute(name: string): boolean;
      setAttribute(name: string, value: string): void;
      removeAttribute(name: string): void;
      appendChild(node: MockCell): MockCell;
      append(...nodes: MockCell[]): void;
      replaceChildren(...nodes: MockCell[]): void;
      contains(node: unknown): boolean;
      focus(): void;
      remove(): void;
    }
    const makeNode = (isFrag = false): MockCell => {
      if (!isFrag) createdElements++;
      const node: MockCell = {
        textContent: '',
        tabIndex: -1,
        dataset: {},
        attrs: new Map(),
        children: [],
        get lastElementChild() { return this.children[this.children.length - 1] ?? null; },
        getAttribute(name) { return this.attrs.get(name) ?? null; },
        hasAttribute(name) { return this.attrs.has(name); },
        setAttribute(name, value) { this.attrs.set(name, value); },
        removeAttribute(name) { this.attrs.delete(name); },
        appendChild(child) {
          const items = child.attrs.get('__frag') === '1' ? child.children : [child];
          for (const item of items) {
            this.children.push(item);
            item.remove = () => {
              const idx = this.children.indexOf(item);
              if (idx >= 0) this.children.splice(idx, 1);
            };
          }
          return child;
        },
        append(...nodes) { for (const n of nodes) this.appendChild(n); },
        replaceChildren(...nodes) { this.children = []; for (const n of nodes) this.appendChild(n); },
        contains() { return false; },
        focus() { (globalThis as unknown as { document: { activeElement: unknown } }).document.activeElement = node; },
        remove() {},
      };
      if (isFrag) node.attrs.set('__frag', '1');
      return node;
    };
    (globalThis as { document: unknown }).document = {
      createElement: () => makeNode(false),
      createDocumentFragment: () => makeNode(true),
      activeElement: null,
    };
    const body = makeNode();
    const opts = {
      selectedKey: null,
      rowsFocusable: true,
      focusedKeyRef: { current: null as string | null },
      focusInsideRef: { current: false },
      restoringFocusRef: { current: false },
      lastMirrorKeyRef: { current: '' },
    };
    syncTableMirror(body as unknown as HTMLTableSectionElement, {
      first: 0,
      total: 2,
      rows: [
        { key: 'GER', name: 'Germany', rowIndex: 0, rank: 1, cells: [{ column: 'name', spoken: 'Germany' }, { column: 'val', spoken: '100' }] },
        { key: 'FRA', name: 'France', rowIndex: 1, rank: 2, cells: [{ column: 'name', spoken: 'France' }, { column: 'val', spoken: '80' }] },
      ],
    }, opts);
    const createdFirst = createdElements;
    const tr0 = body.children[0]!;
    const td2 = tr0.children[2]!;
    assert.equal(td2.textContent, '100');

    syncTableMirror(body as unknown as HTMLTableSectionElement, {
      first: 0,
      total: 2,
      rows: [
        { key: 'GER', name: 'Germany', rowIndex: 0, rank: 1, cells: [{ column: 'name', spoken: 'Germany' }, { column: 'val', spoken: '105' }] },
        { key: 'FRA', name: 'France', rowIndex: 1, rank: 2, cells: [{ column: 'name', spoken: 'France' }, { column: 'val', spoken: '80' }] },
      ],
    }, opts);
    assert.equal(createdElements, createdFirst, 'updating values reuses all existing tr and td nodes');
    assert.equal(body.children[0], tr0);
    assert.equal(tr0.children[2], td2);
    assert.equal(td2.textContent, '105');

    syncTableMirror(body as unknown as HTMLTableSectionElement, {
      first: 0,
      total: 1,
      rows: [{ key: 'GER', name: 'Germany', rowIndex: 0, rank: 1, cells: [{ column: 'name', spoken: 'Germany' }, { column: 'val', spoken: '105' }] }],
    }, opts);
    assert.equal(body.children.length, 1, 'shrinking rows trims trailing tr nodes');

    const rows = (keys: readonly string[]) => ({ first: 0, total: keys.length, rows: keys.map((key, rowIndex) => ({ key, name: key, rowIndex, cells: [] })) });
    syncTableMirror(body as unknown as HTMLTableSectionElement, rows(['GER', 'FRA']), { ...opts, focusedKeyRef: { current: 'GER' } });
    const focusedRef = { current: 'GER' as string | null };
    const prevElement = (globalThis as { HTMLElement?: unknown }).HTMLElement;
    (globalThis as { HTMLElement?: unknown }).HTMLElement = Object;
    body.contains = () => true;
    body.children[0]!.focus();
    const moved = syncTableMirror(body as unknown as HTMLTableSectionElement, rows(['FRA']), { ...opts, focusedKeyRef: focusedRef, focusInsideRef: { current: true } });
    assert.equal(moved, true, 'when the focused row goes, the mirror reports that it moved focus');
    assert.equal(focusedRef.current, 'FRA', 'and the host focus key follows the row that took DOM focus');
    (globalThis as { HTMLElement?: unknown }).HTMLElement = prevElement;

    const bodyFocus = makeNode();
    syncTableMirror(bodyFocus as unknown as HTMLTableSectionElement, rows(['GER', 'FRA']), { ...opts, rowsFocusable: false, lastMirrorKeyRef: { current: '' } });
    assert.ok(bodyFocus.children.every(tr => !tr.hasAttribute('tabindex') && tr.tabIndex === -1), 'a mirror of a body-focused family has no tabbable rows outside the keyboard handler');
  } finally {
    (globalThis as { document: unknown }).document = prevDoc;
  }
});

test('bindFamilyPointer resolves actions against the installed picture, rejects stale owners, separates click from drag pan, and keeps continuous hover flicker-free', () => {
  type Listener = (event: unknown) => void;
  const makeTarget = () => {
    const listeners = new Map<string, Set<Listener>>();
    return {
      addEventListener(type: string, fn: Listener) {
        let set = listeners.get(type);
        if (!set) listeners.set(type, set = new Set());
        set.add(fn);
      },
      removeEventListener(type: string, fn: Listener) {
        listeners.get(type)?.delete(fn);
      },
      dispatch(type: string, event: Record<string, unknown>) {
        for (const fn of listeners.get(type) ?? []) fn(event);
      },
      getBoundingClientRect: () => ({ left: 10, top: 20, width: 400, height: 200 }),
      contains: () => false,
    };
  };

  const body = makeTarget();
  const menuHost = makeTarget();
  let pictureBox = body.getBoundingClientRect();
  const picture = { getBoundingClientRect: () => pictureBox };
  const nav: readonly FamilyNavTuple[] = [['GER', 'Germany', 0, -1, '', { tag: 'GER' }]];
  let installed: InstalledPicture | null = {
    id: 7,
    owner: 'owner-A',
    generation: { seek: 1, reset: 0, binding: 1 },
    factRevisions: {},
    validInterval: [100, 120],
    through: 100,
    hour: displayedHour(100),
    viewport: { width: 200, height: 100, dpr: 1, scrollLeft: 0, scrollTop: 0 },
    optionsRevision: '',
    themeRevision: '',
    actionKeys: lazyActionKeys(nav, displayedHour(100)),
    accessibleDescription: null,
  };

  let hitReply: { key: string | null; meta?: PointerHitMeta } = { key: 'row:GER', meta: { installedId: 7, owner: 'owner-A' } };
  const inspectedTokens: (string | null)[] = [];
  const pointerPoints: (readonly [number, number] | null)[] = [];
  const fakePipeline = {
    pointer(_pipe: number, _slot: number, pt: readonly [number, number] | null, cb?: (key: string | null, meta?: PointerHitMeta) => void) {
      pointerPoints.push(pt);
      cb?.(hitReply.key, hitReply.meta);
    },
    inspect(_pipe: number, token: string | null) {
      inspectedTokens.push(token);
    },
    view() { return null; },
  } as unknown as Pipeline;

  const tooltips = new FamilyTooltipController('test-host');
  const activations: InstalledAction[] = [];
  const hoverHits: (ParsedFamilyHit | null)[] = [];
  const dragEvents: string[] = [];
  let doubleClicks = 0;
  let expired = false;

  const stop = bindFamilyPointer({
    bodyNode: body as unknown as HTMLElement,
    pictureNode: picture as unknown as HTMLElement,
    menuHost: menuHost as unknown as HTMLElement,
    pipeline: fakePipeline,
    pipe: 1,
    tooltips,
    inspectedKeyRef: { current: null },
    inspectedHitRef: { current: null },
    lastPointerRef: { current: null },
    focusInsideRef: { current: false },
    focusedKeyRef: { current: null },
    focusAnchorRef: { current: { x: 0, y: 0 } },
    accessibleRef: { current: null },
    pointerMode: 'continuous',
    isBlocked: () => !installed || expired,
    getInstalled: () => installed,
    getViewportHeight: () => 800,
    parseHit: (hitKey, inst) => parseTimeChartHit(hitKey) ?? parseDefaultRowHit(hitKey) ?? (inst ? null : null),
    onHoverChange: hit => hoverHits.push(hit),
    gestures: {
      onDragStart: () => dragEvents.push('start'),
      onDragMove: (_pt, delta) => dragEvents.push(`move:${delta[0]},${delta[1]}`),
      onDragEnd: () => dragEvents.push('end'),
      onDoubleClick: () => { doubleClicks++; return true; },
    },
    onActivate: (key, _toggle, hit, picture) => { if (picture) activations.push({ installedId: picture.id, owner: picture.owner, subject: hit?.subject ?? key, hour: displayedHour(hit?.hour ?? picture.hour) }); },
  });

  body.dispatch('pointerdown', { button: 0, clientX: 110, clientY: 70, pointerId: 1 });
  body.dispatch('pointerup', { button: 0, clientX: 110, clientY: 70, pointerId: 1 });
  assert.deepEqual(pointerPoints[0], [50, 25], 'client point scales through installed viewport (200x100 vs DOM 400x200)');
  assert.equal(activations.length, 1);
  assert.equal(activations[0]!.installedId, 7);
  assert.equal(activations[0]!.owner, 'owner-A');
  assert.equal(activations[0]!.subject, 'GER');
  assert.equal(activations[0]!.hour, 100);

  hitReply = { key: 'row:GER', meta: { installedId: 6, owner: 'owner-OLD' } };
  body.dispatch('pointerdown', { button: 0, clientX: 110, clientY: 70, pointerId: 1 });
  body.dispatch('pointerup', { button: 0, clientX: 110, clientY: 70, pointerId: 1 });
  assert.equal(activations.length, 1, 'stale owner/installedId hit is rejected');

  hitReply = { key: 'plot:150', meta: { installedId: 7, owner: 'owner-A' } };
  body.dispatch('pointerdown', { button: 0, clientX: 100, clientY: 70, pointerId: 2 });
  body.dispatch('pointermove', { isTrusted: true, clientX: 100 + CLICK_CANCEL_PX + 10, clientY: 70, pointerId: 2 });
  body.dispatch('pointerup', { button: 0, clientX: 100 + CLICK_CANCEL_PX + 10, clientY: 70, pointerId: 2 });
  assert.equal(activations.length, 1, 'drag past CLICK_CANCEL_PX cancels click activation');
  assert.deepEqual(dragEvents, ['start', `move:${CLICK_CANCEL_PX + 10},0`, 'end']);

  dragEvents.length = 0;
  body.dispatch('pointerdown', { button: 0, clientX: 100, clientY: 70, pointerId: 3 });
  body.dispatch('pointermove', { isTrusted: true, clientX: 100 + CLICK_CANCEL_PX + 10, clientY: 70, pointerId: 3 });
  const panned = installed;
  installed = { ...installed!, id: 8, owner: 'owner-B' };
  body.dispatch('pointermove', { isTrusted: true, clientX: 100 + CLICK_CANCEL_PX + 30, clientY: 70, pointerId: 3 });
  body.dispatch('pointerup', { button: 0, clientX: 100 + CLICK_CANCEL_PX + 30, clientY: 70, pointerId: 3 });
  assert.deepEqual(dragEvents, ['start', `move:${CLICK_CANCEL_PX + 10},0`, 'end'], 'R4-S9: a pan that meets an owner change ends there, and the replacement chart never gets the old owner’s range');
  dragEvents.length = 0;
  installed = panned;
  const blockedBefore = expired;
  body.dispatch('pointerdown', { button: 0, clientX: 100, clientY: 70, pointerId: 4 });
  expired = true;
  body.dispatch('pointermove', { isTrusted: true, clientX: 100 + CLICK_CANCEL_PX + 10, clientY: 70, pointerId: 4 });
  body.dispatch('pointerup', { button: 0, clientX: 100 + CLICK_CANCEL_PX + 10, clientY: 70, pointerId: 4 });
  assert.deepEqual([blockedBefore, dragEvents], [false, []], 'R4-S9: a pan does not start on an expired picture');
  expired = false;

  let dblPrevented = false;
  body.dispatch('dblclick', { clientX: 100, clientY: 70, preventDefault() { dblPrevented = true; } });
  assert.equal(doubleClicks, 1);
  assert.equal(dblPrevented, true);

  hitReply = { key: 'line:GER@150', meta: { installedId: 7, owner: 'owner-A' } };
  body.dispatch('pointermove', { isTrusted: true, clientX: 120, clientY: 80 });
  hitReply = { key: 'line:GER@151', meta: { installedId: 7, owner: 'owner-A' } };
  body.dispatch('pointermove', { isTrusted: true, clientX: 122, clientY: 80 });
  assert.deepEqual(hoverHits.slice(-2).map(h => h?.hour), [150, 151]);
  assert.equal(inspectedTokens.at(-1), 'line:GER@151');

  const tip150: FamilyTooltipEntry = { hour: 150, rowKey: 'line:GER@150', payload: { title: 'Germany', hour: 150 } };
  tooltips.syncFromInstall({
    dim: false,
    held: false,
    inspectedKey: 'line:GER@150',
    inspectedToken: 'line:GER@150',
    pointerPos: { x: 120, y: 80 },
    hoverTip: tip150,
    hoverFieldKey: 'line:GER@150',
    keepHoverWhilePending: true,
    focusInside: false,
    focusedKey: null,
    focusAnchor: { x: 0, y: 0 },
    focusTip: null,
    viewportHeight: 800,
  });
  assert.equal(tooltipStore.get()?.content.title, 'Germany');
  tooltips.syncFromInstall({
    dim: false,
    held: false,
    inspectedKey: 'line:GER@151',
    inspectedToken: 'line:GER@151',
    pointerPos: { x: 122, y: 80 },
    hoverTip: tip150,
    hoverFieldKey: 'line:GER@150',
    keepHoverWhilePending: true,
    focusInside: false,
    focusedKey: null,
    focusAnchor: { x: 0, y: 0 },
    focusTip: null,
    viewportHeight: 800,
  });
  assert.equal(tooltipStore.get()?.content.title, 'Germany', 'continuous hover keeps previous tooltip while adjacent hour renders');
  tooltips.syncFromInstall({
    dim: false,
    held: false,
    inspectedKey: 'line:GER@151',
    inspectedToken: 'line:GER@151',
    pointerPos: { x: 122, y: 80 },
    hoverTip: null,
    hoverFieldKey: 'line:GER@151',
    keepHoverWhilePending: true,
    focusInside: false,
    focusedKey: null,
    focusAnchor: { x: 0, y: 0 },
    focusTip: null,
    viewportHeight: 800,
  });
  assert.equal(tooltipStore.get(), null, 'explicit null tooltip for current token hides immediately');

  stop.clearHover();
  assert.equal(inspectedTokens.at(-1), null, 'a zoom or pan drops the inspected hour under a still pointer');
  assert.equal(hoverHits.at(-1), null, 'and publishes no hover hour');

  pictureBox = { left: 10, top: 20, width: 200, height: 100 };
  body.dispatch('pointermove', { isTrusted: true, clientX: 110, clientY: 70 });
  assert.deepEqual(pointerPoints.at(-1), [100, 50], 'a picture shown at a size other than its body, as one held across a scale change is, maps the pointer by its own box');

  stop();
  tooltips.dispose();
});

test('grid and points keyboard navigation and family host adapters cover all four ported families', () => {
  const cells: string[] = [];
  const gridActivated: string[] = [];
  handleFamilyGridKeyDown(mkEvent('ArrowRight').event, {
    rows: 3,
    columns: 4,
    current: { row: 1, column: 1 },
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusCell: (r, c) => cells.push(`${r}:${c}`),
    onActivate: (r, c) => gridActivated.push(`${r}:${c}`),
  });
  handleFamilyGridKeyDown(mkEvent('End', { ctrlKey: true }).event, {
    rows: 3,
    columns: 4,
    current: { row: 1, column: 1 },
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusCell: (r, c) => cells.push(`${r}:${c}`),
    onActivate: (r, c) => gridActivated.push(`${r}:${c}`),
  });
  handleFamilyGridKeyDown(mkEvent('Enter').event, {
    rows: 3,
    columns: 4,
    current: { row: 2, column: 3 },
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusCell: (r, c) => cells.push(`${r}:${c}`),
    onActivate: (r, c) => gridActivated.push(`${r}:${c}`),
  });
  assert.deepEqual(cells, ['1:2', '2:3']);
  assert.deepEqual(gridActivated, ['2:3']);

  const pointsNav: FamilyNavTuple[] = [
    ['plot:100', 'Jan 1936', 0, -1, ''],
    ['plot:200', 'Feb 1936', 1, -1, ''],
    ['plot:300', 'Mar 1936', 2, -1, ''],
  ];
  const focusedPts: string[] = [];
  const activatedPts: string[] = [];
  handleFamilyPointsKeyDown(mkEvent('ArrowRight').event, {
    nav: pointsNav,
    focusedKey: 'plot:100',
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: key => focusedPts.push(key),
    onActivate: entry => activatedPts.push(entry[0]),
  });
  handleFamilyPointsKeyDown(mkEvent('End').event, {
    nav: pointsNav,
    focusedKey: 'plot:100',
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: key => focusedPts.push(key),
    onActivate: entry => activatedPts.push(entry[0]),
  });
  handleFamilyPointsKeyDown(mkEvent('Enter').event, {
    nav: pointsNav,
    focusedKey: 'plot:200',
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: key => focusedPts.push(key),
    onActivate: entry => activatedPts.push(entry[0]),
  });
  assert.deepEqual(focusedPts, ['plot:200', 'plot:300']);
  assert.deepEqual(activatedPts, ['plot:200']);

  assert.deepEqual(parseBarsHit('right:GER'), { key: 'GER', subject: 'GER', kind: 'right-row', toggle: false, inspectToken: 'right:GER' });
  assert.equal(parseBarsHit('remove:reg1'), null, 'removing a region is the regions control, never a canvas hit');
  assert.deepEqual(parseHeatMatrixHit('cell:2:5'), { key: '2:5', subject: '2:5', toggle: false, inspectToken: 'cell:2:5' });
  assert.equal(parseHeatLanesHit('lane:#0@144')?.hour, 144);
  assert.equal(parseTimeChartHit('line:GER@512')?.hour, 512);
  assert.equal(parseTimeChartHit('line:GER@512')?.subject, 'GER');
  assert.equal(barsHostAdapter.family, 'bars');
  assert.equal(heatMatrixHostAdapter.family, 'heat-matrix');
  assert.equal(heatLanesHostAdapter.family, 'heat-lanes');
  assert.equal(timeChartHostAdapter.family, 'time-chart');

  let zoomWindow: readonly [number, number] | null = null;
  const gCtx = {
    fields: { meta: { plotRect: { left: 50, top: 10, width: 400, height: 200 }, transform: { xFrom: 0, xTo: 1000, axisFrom: 0, axisTo: 1000 } } },
    viewport: { width: 500, height: 240, dpr: 1, scrollLeft: 0, scrollTop: 0, zoomWindow: null as readonly [number, number] | null },
    extent: [0, 1000] as readonly [number, number],
    setZoomWindow: (next: readonly [number, number] | null) => { zoomWindow = next; },
  };
  assert.equal(timeChartHostAdapter.gestures!.onWheel!({ deltaY: -120 } as WheelEvent, [250, 100], gCtx), true);
  assert.ok(zoomWindow !== null && zoomWindow[1] - zoomWindow[0] < 1000, 'wheel in zooms window');
  assert.equal(timeChartHostAdapter.gestures!.onDoubleClick!([250, 100], { ...gCtx, viewport: { ...gCtx.viewport, zoomWindow } }), true);
  assert.equal(zoomWindow, null, 'double-click resets zoom window');
});


test('a press, release or menu waits for its own hit and acts only on the picture that hit, with a delayed worker', () => {
  type Listener = (event: unknown) => void;
  const listeners = new Map<string, Set<Listener>>();
  const body = {
    addEventListener(type: string, fn: Listener) { (listeners.get(type) ?? listeners.set(type, new Set()).get(type)!).add(fn); },
    removeEventListener(type: string, fn: Listener) { listeners.get(type)?.delete(fn); },
    dispatch(type: string, event: Record<string, unknown>) { for (const fn of listeners.get(type) ?? []) fn({ target: body, preventDefault() {}, ...event }); },
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 200, height: 100 }),
    contains: (node: unknown) => node === body,
  };
  const picture = (id: number, keys: readonly string[], owner = 'owner-A'): InstalledPicture => ({
    id,
    owner,
    generation: { seek: 1, reset: 0, binding: 1 },
    factRevisions: {},
    validInterval: [100, 120],
    through: 100,
    hour: displayedHour(100 + id),
    viewport: { width: 200, height: 100, dpr: 1, scrollLeft: 0, scrollTop: 0 },
    optionsRevision: '',
    themeRevision: '',
    actionKeys: lazyActionKeys(keys.map((key, i) => [key, key, i, -1, '', null] as FamilyNavTuple), displayedHour(100 + id)),
    accessibleDescription: null,
  });
  let installed = picture(7, ['GER']);
  const pending: ((key: string | null, meta?: PointerHitMeta) => void)[] = [];
  const fakePipeline = {
    pointer(_pipe: number, _slot: number, _pt: unknown, cb?: (key: string | null, meta?: PointerHitMeta) => void) { if (cb) pending.push(cb); },
    inspect(_pipe: number, token: string | null) { inspected.push(token); },
    view() { return null; },
  } as unknown as Pipeline;
  const inspected: (string | null)[] = [];
  const reply = (key: string | null, id: number, owner = 'owner-A', at = 0) => pending.splice(at, 1)[0]!(key, { installedId: id, owner });
  const activations: InstalledAction[] = [];
  const menus: (string | null)[] = [];
  const inspectedKeyRef = { current: null as string | null };
  const tooltips = new FamilyTooltipController('delayed-worker');
  const stop = bindFamilyPointer({
    bodyNode: body as unknown as HTMLElement,
    pictureNode: body as unknown as HTMLElement,
    menuHost: body as unknown as HTMLElement,
    pipeline: fakePipeline,
    pipe: 1,
    tooltips,
    inspectedKeyRef,
    inspectedHitRef: { current: null },
    lastPointerRef: { current: null },
    focusInsideRef: { current: false },
    focusedKeyRef: { current: null },
    focusAnchorRef: { current: { x: 0, y: 0 } },
    accessibleRef: { current: body as unknown as HTMLElement },
    isBlocked: () => false,
    getInstalled: () => installed,
    getViewportHeight: () => 800,
    parseHit: hit => parseDefaultRowHit(hit),
    onActivate: (key, _toggle, hit, picture) => { if (picture) activations.push({ installedId: picture.id, owner: picture.owner, subject: hit?.subject ?? key, hour: displayedHour(hit?.hour ?? picture.hour) }); },
    onContextMenu: key => { menus.push(key); return true; },
  });

  body.dispatch('pointermove', { isTrusted: true, clientX: 20, clientY: 10 });
  reply('row:GER', 7);
  assert.equal(inspectedKeyRef.current, 'GER', 'GER hovered on picture 7');

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 1 });
  installed = picture(8, ['FRA']);
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 1 });
  assert.equal(activations.length, 0, 'the cached hover is not a hit: nothing activates before this press resolves');
  reply('row:GER', 7);
  reply('row:FRA', 8);
  assert.equal(activations.length, 0, 'press on GER in picture 7 and release on FRA in picture 8 is not a click');

  body.dispatch('contextmenu', { clientX: 20, clientY: 10 });
  assert.deepEqual(menus, [], 'the menu waits for its own hit');
  installed = picture(9, ['ITA']);
  reply('row:FRA', 8);
  assert.deepEqual(menus, ['FRA'], 'a menu answers from the picture it was opened on, though the same owner repainted since');
  body.dispatch('contextmenu', { clientX: 20, clientY: 10 });
  reply('row:ITA', 9);
  assert.deepEqual(menus, ['FRA', 'ITA']);

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 2 });
  reply('row:ITA', 9);
  installed = picture(10, ['ENG']);
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 2 });
  reply('row:ENG', 10);
  assert.equal(activations.length, 0, 'press on ITA and release on ENG is not a click');

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 3 });
  reply('row:ENG', 10);
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 3 });
  installed = picture(11, ['ENG']);
  reply('row:ENG', 10);
  assert.equal(activations.length, 1, 'a valid release hit is kept when the same owner repaints before it arrives');
  assert.equal(activations[0]!.installedId, 10, 'and the action belongs to the picture the release hit');
  assert.equal(activations[0]!.hour, 110);

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 4 });
  reply('row:ENG', 11);
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 4 });
  reply('row:ENG', 11);
  assert.equal(activations.length, 2);
  assert.equal(activations[1]!.installedId, 11);
  assert.equal(activations[1]!.subject, 'ENG');
  assert.equal(activations[1]!.hour, 111);

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 5 });
  reply('row:ENG', 11);
  installed = picture(12, ['ENG'], 'owner-B');
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 5 });
  assert.equal(pending.length, 0, 'a press on owner A does not authorise a release on owner B');
  assert.equal(activations.length, 2);

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 6 });
  reply('row:ENG', 12, 'owner-B');
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 6 });
  installed = picture(13, ['ENG'], 'owner-C');
  reply('row:ENG', 12, 'owner-B');
  assert.equal(activations.length, 2, 'an owner change before the release hit arrives refuses it');

  body.dispatch('pointermove', { isTrusted: true, clientX: 24, clientY: 10 });
  reply('row:GER', 13, 'owner-C');
  assert.equal(inspectedKeyRef.current, 'GER');
  body.dispatch('pointerdown', { button: 0, clientX: 24, clientY: 10, pointerId: 7 });
  installed = picture(14, ['FRA', 'GER'], 'owner-C');
  body.dispatch('pointermove', { isTrusted: true, clientX: 25, clientY: 10 });
  reply('row:FRA', 14, 'owner-C', 1);
  assert.equal(inspectedKeyRef.current, 'FRA', 'FRA moved under the pointer and is the hover of picture 14');
  inspected.length = 0;
  reply('row:GER', 13, 'owner-C');
  assert.equal(inspectedKeyRef.current, 'FRA', 'R4-S7: the late press reply keeps GER for click matching but does not publish it as the hover');
  assert.deepEqual(inspected, [], 'R4-S7: and does not inspect GER in the lane');
  body.dispatch('pointerup', { button: 0, clientX: 25, clientY: 10, pointerId: 7 });
  reply('row:FRA', 14, 'owner-C');
  assert.equal(activations.length, 2, 'press on GER and release on FRA is still not a click');

  stop();
  tooltips.dispose();
});

test('a followed bounded read dates its picture by its proof even when the widget also has subscriptions', () => {
  const followed = { hour: 120, follows: true };
  assert.deepEqual(boundDated(followed, [100, 140], true, false, null), { h: 100, c: 140 }, 'held back between a seek back and the new bounded reply');
  assert.equal(boundDated({ hour: 120 }, null, true, false, null), undefined, 'a pinned read beside subscriptions is dated by the stream');
  assert.deepEqual(boundDated({ hour: 120 }, null, false, false, null), { h: 120, c: 120 });
  assert.deepEqual(boundDated(null, null, false, true, [0, 500]), { h: 0, c: 500 });
});

test('R3-S1: a paged table scrolled past its head asks the absolute rows under the viewport before projection, of the lane, the page broker and the exact audit read alike', async () => {
  const source = { id: 'log-src', revision: 'r1', path: 'p', metadata: { branches: [{ id: '1', firstHour: 0, lastHour: 5000 }] } } as unknown as DashboardSource;
  const HEAD = 64;
  const TOTAL = 1000;
  const entry = pagedTailEntry('main', source, '1', 0, { product: 'event-choices-tail', branchId: '1', facets: [], from: null, limit: HEAD }, true);
  const plan: SourcePlan = { owner: 'log', entries: [entry] };
  const columns = [{ key: 'event', label: 'Event', as: 'plain', offered: ['plain'], priority: 0 }] as never;
  const settings = tableSettingsOf(undefined, columns, { density: 'normal', sort: { column: 'event', direction: 'descending' } });
  const rowHeight = ROW_HEIGHT.normal;
  const installed = { table: { count: TOTAL, totalHeight: TOTAL * rowHeight }, meta: {} };
  const viewport = { width: 600, height: 10 * rowHeight, dpr: 1, scrollLeft: 0, scrollTop: 600 * rowHeight, zoomWindow: null };

  const window = requestedRowWindow(tableHostAdapter, plan, viewport, settings, installed);
  assert.deepEqual(window, { first: 596, count: 18 }, 'the ordinals under the scroll position with overscan, far beyond the 64-row head');
  assert.equal(requestedRowWindow(tableHostAdapter, { owner: 'log', entries: [] }, viewport, settings, installed), null, 'a table with no paged read asks no window');
  assert.deepEqual(
    requestedRowWindow(tableHostAdapter, plan, { ...viewport, scrollTop: (TOTAL - 10) * rowHeight }, settings, installed),
    { first: 986, count: 14 },
    'the deepest scroll asks the last rows the recording holds, not past them',
  );

  const ctx: FamilyBindingContext<unknown, typeof settings> = {
    kind: 'event-choices', view: 'default', options: {}, settings, sources: {}, extent: null, plan, viewport,
    theme: {} as never, reducedMotion: false, selectedKey: null, focusedKey: null, expanded: {}, audit: false, auditId: undefined, pauseFlush: 0,
    window,
  };
  assert.deepEqual((familyBinding(tableHostAdapter, ctx) as { window?: unknown } | null)?.window, window, 'the lane is bound to the requested window, so the projection reads those rows');

  const asked: [number, number][] = [];
  const client = {
    product: async (request: { params: Record<string, unknown> }) => {
      const offset = Number(request.params.offset ?? 0);
      const limit = Number(request.params.limit);
      asked.push([offset, limit]);
      const rows = Array.from({ length: Math.max(0, Math.min(limit, TOTAL - offset)) }, (_, i) => ({ key: `e${offset + i}`, hour: 100 }));
      return { data: { total: String(TOTAL), offset: String(offset), baseOrdinal: '0', firstOrdinal: '0', rows, missing: [], facetTotals: [] } };
    },
  } as unknown as DashboardClient;
  const gate: FrozenGate = { recorded: (h: number | null | undefined) => (typeof h === 'number' ? h : null) as never, span: (firstHour: number, lastHour: number) => ({ firstHour, lastHour } as never) };
  const exact = await readPlanExact(plan, 200, { client, following: false, timeMode: 'pin', branch: { firstHour: 0, lastHour: 5000 }, window: null, staticSpan: null, gate, rowWindow: window });
  assert.deepEqual(asked, [[2 * DEFAULT_TAIL_PAGE_SIZE, DEFAULT_TAIL_PAGE_SIZE]], 'the exact read of a scrolled window takes only the snapshot page holding rows 596 to 613, never the head');
  assert.deepEqual(exact?.rowWindow, window, 'and projects the same window the lane painted');
  assert.deepEqual(exact?.rows('main')?.sliceAt(200, 596, 2).map(slot => (slot.kind === 'data' ? slot.row.key : slot.kind)), ['e596', 'e597']);
  const snapshotHours: unknown[] = [];
  const atSnapshot = { product: async (request: { params: Record<string, unknown> }) => { snapshotHours.push(request.params.hour); return client.product(request as never, new AbortController().signal); } } as unknown as DashboardClient;
  const atS = await readPlanExact(plan, 200, { client: atSnapshot, following: false, timeMode: 'pin', branch: { firstHour: 0, lastHour: 5000 }, window: null, staticSpan: null, gate, rowWindow: window, snapshotHour: () => 150 });
  assert.deepEqual(snapshotHours, [150], 'a scrolled source is audited at the hour of the snapshot the picture shows');
  assert.equal(atS?.rows('main')?.at, 150);

  const model: TableModel = { columns, hour: 200, rows: Array.from({ length: 18 }, (_, i) => ({ key: `e${596 + i}`, name: `e${596 + i}`, cells: { event: { value: { kind: 'known', value: `e${596 + i}` } } } })) as never, windowOffset: 596, totalRows: TOTAL };
  const audited = tableHostAdapter.deriveAuditRows!(model, 200, { viewport, settings, expanded: {}, theme: undefined, fields: { ...installed, range: { first: 4, count: 10 } } });
  assert.deepEqual(
    [...new Set(audited.map(row => row.key.split(':')[0]!).filter(key => key.startsWith('e')))].sort(),
    Array.from({ length: 10 }, (_, i) => `e${600 + i}`),
    'the audit covers the painted local slice the installed picture names, not rows found from the absolute scroll offset',
  );
});

test('class D: every action and published state comes from the installed picture record the input was aimed at (R3-S5, R3-S13, R3-S9, R3-S4)', () => {
  type Listener = (event: unknown) => void;
  const listeners = new Map<string, Set<Listener>>();
  const body = {
    addEventListener(type: string, fn: Listener) { (listeners.get(type) ?? listeners.set(type, new Set()).get(type)!).add(fn); },
    removeEventListener(type: string, fn: Listener) { listeners.get(type)?.delete(fn); },
    dispatch(type: string, event: Record<string, unknown>) { for (const fn of listeners.get(type) ?? []) fn({ target: body, preventDefault() {}, ...event }); },
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 200, height: 100 }),
    contains: (node: unknown) => node === body,
    setPointerCapture() {},
    releasePointerCapture() {},
  };
  const nav = (rows: readonly (readonly [string, number])[]): FamilyNavTuple[] => rows.map(([key, tree], i) => [key, `${key} name`, i, tree, '', null] as FamilyNavTuple);
  const picture = (id: number, rows: readonly (readonly [string, number])[], owner = 'owner-A'): InstalledPicture => ({
    id, owner, generation: { seek: 1, reset: 0, binding: 1 }, factRevisions: {}, validInterval: [100, 120], through: 100, hour: displayedHour(100 + id),
    viewport: { width: 200, height: 100, dpr: 1, scrollLeft: 0, scrollTop: 0 }, optionsRevision: '', themeRevision: '',
    actionKeys: lazyActionKeys(nav(rows), displayedHour(100 + id)), accessibleDescription: null,
  });

  let installed = picture(7, [['A', -1], ['B', -1]]);
  const pending: ((key: string | null, meta?: PointerHitMeta) => void)[] = [];
  const fakePipeline = {
    pointer(_pipe: number, _slot: number, _pt: unknown, cb?: (key: string | null, meta?: PointerHitMeta) => void) { if (cb) pending.push(cb); },
    inspect() {},
    view() { return null; },
  } as unknown as Pipeline;
  const reply = (key: string | null, id: number, owner = 'owner-A') => pending.shift()!(key, { installedId: id, owner });
  const inspectedKeyRef = { current: null as string | null };
  const drags: string[] = [];
  const tooltips = new FamilyTooltipController('class-d');
  const stop = bindFamilyPointer({
    bodyNode: body as unknown as HTMLElement,
    pictureNode: body as unknown as HTMLElement,
    menuHost: body as unknown as HTMLElement,
    pipeline: fakePipeline,
    pipe: 1,
    tooltips,
    inspectedKeyRef,
    inspectedHitRef: { current: null },
    lastPointerRef: { current: null },
    focusInsideRef: { current: false },
    focusedKeyRef: { current: null },
    focusAnchorRef: { current: { x: 0, y: 0 } },
    accessibleRef: { current: body as unknown as HTMLElement },
    isBlocked: () => false,
    getInstalled: () => installed,
    getViewportHeight: () => 800,
    parseHit: hit => parseDefaultRowHit(hit),
    rowDrag: {
      starts: hit => { drags.push(`start ${hit.key}`); return true; },
      move: () => { drags.push('move'); },
      end: point => { drags.push(point ? 'drop' : 'cancel'); },
    },
    onActivate: () => {},
  });

  body.dispatch('pointermove', { isTrusted: true, clientX: 20, clientY: 10 });
  installed = picture(8, [['B', -1], ['A', -1]]);
  reply('row:A', 7);
  assert.equal(inspectedKeyRef.current, null, 'R3-S13: a hover reply for a replaced picture inspects nothing');
  assert.equal(pending.length, 1, 'and the last pointer position is hit again on the current picture');
  reply('row:B', 8);
  assert.equal(inspectedKeyRef.current, 'B', 'so the row now under the stationary pointer is inspected');

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 1 });
  reply('row:B', 8);
  body.dispatch('pointermove', { isTrusted: true, clientX: 20, clientY: 40 });
  installed = picture(9, [['B', -1]], 'owner-B');
  body.dispatch('pointermove', { isTrusted: true, clientX: 20, clientY: 60 });
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 60, pointerId: 1 });
  assert.deepEqual(drags, ['start B', 'move', 'cancel'], 'R3-S5: a drag begun on owner A is cancelled when the owner changes, never dropped on owner B');
  stop();
  tooltips.dispose();

  const adapter = { family: 'regions-test', navigation: 'rows' } as FamilyHostAdapter;
  const p1 = picture(20, [['GER', 0], ['ITA', -1]]);
  const p2 = picture(21, [['ITA', -1], ['GER', 1]]);
  const r1 = recordInstalled(p1, { nav: nav([['GER', 0], ['ITA', -1]]), regions: { rows: [{ key: 'r1' }], add: { top: 40, height: 20 }, nameLeft: 8 } }, 'GER');
  const r2 = recordInstalled(p2, { nav: nav([['ITA', -1], ['GER', 1]]), regions: { rows: [{ key: 'r1' }, { key: 'r2' }], add: { top: 60, height: 20 }, nameLeft: 8 } }, 'ITA');
  assert.equal(installedRecordOf(p1), r1);
  const gate = (current: typeof r2 | null, expired = false): HostInputGate => ({ current, held: false, expired });
  const options = { expansion: {}, takesToggles: false, regions: true, menu: true };
  const grid = () => null;
  const addRegion = resolveHostInput(adapter, 'activate', { key: '__add-region', hit: { key: '__add-region', kind: 'region-add' }, picture: p1 }, gate(r2), options, grid);
  assert.deepEqual(addRegion?.kind === 'region-open' ? addRegion.field : null, r1.fields.regions, 'R3-S5: a delayed Add region click opens the region list of the picture it hit, not the replacement');
  const toggled = resolveHostInput(adapter, 'toggle', { key: 'GER', hit: null, picture: p1 }, gate(r2), options, grid);
  assert.deepEqual(toggled?.kind === 'toggle' ? [toggled.action.installedId, toggled.expansion?.open, toggled.expansion?.next] : null, [20, true, { GER: true }], 'a tree toggle opens what the event picture showed closed, though the latest picture shows it open');
  assert.equal(resolveHostInput(adapter, 'activate', { key: 'ITA', hit: null, picture: p1 }, gate(r2, true), options, grid), null, 'R3-S4: Enter on an expired picture does nothing, read at input time');
  assert.equal(resolveHostInput(adapter, 'activate', { key: 'ITA', hit: null, picture: p1 }, gate(recordInstalled(picture(22, [['ITA', -1]], 'owner-B'), {}, null)), options, grid), null, 'an input aimed at owner A never acts once owner B is installed');
  const cardPicture = picture(23, [['GER', -1]]);
  const cardRecord = recordInstalled(cardPicture, { nav: nav([['GER', -1]]), rowHours: { GER: 9 } }, 'GER');
  const menu = resolveHostInput(cardHostAdapter, 'menu', { key: 'GER', hit: null, picture: cardPicture }, gate(cardRecord), options, grid);
  assert.deepEqual(menu?.kind === 'menu' ? [menu.hit.hour, menu.hit.hitHour] : null, [9, 9], '#967 R2-S1: a keyboard menu on a dated card row acts at the hour of that row, not the hour of the installed picture');
  const pointed = resolveHostInput(cardHostAdapter, 'menu', { key: 'GER', hit: { key: 'GER', hour: 5 }, picture: cardPicture }, gate(cardRecord), options, grid);
  assert.deepEqual(pointed?.kind === 'menu' ? [pointed.hit.hour, pointed.hit.hitHour] : null, [5, 5], 'a pointer hit keeps the hour it carries');

  const selections = new BoundSelections();
  selections.bound(3, 'A');
  selections.bound(5, 'B');
  assert.deepEqual([selections.at(3), selections.at(4), selections.at(5)], ['A', 'A', 'B'], 'R3-S9: the mirror announces the selection the installed picture was bound with');
});

test('R3-S7: Page Up and Page Down each find their destination in their own direction from the installed bounds', () => {
  const nav = Array.from({ length: 50 }, (_, i) => [`r${i}`, `Row ${i}`, i, -1, '', null, i * 20, 20] as FamilyNavTuple);
  assert.deepEqual(navPageSteps(nav, 'r49', 100), { down: 1, up: 4 }, 'from the last row Page Up moves a viewport up, not one row');
  assert.deepEqual(navPageSteps(nav, 'r0', 100), { down: 4, up: 1 });
  assert.deepEqual(navPageSteps(nav, 'r20', 60), { down: 2, up: 2 }, 'a smaller usable row viewport (a bars body under its axis) gives smaller steps');
  assert.equal(usableRowHeight({ meta: { rowViewport: { top: 24, height: 60 } } }, 100), 60, 'a lane that publishes its row viewport pages and reveals within it');
  assert.equal(usableRowHeight({ meta: {} }, 100), 100);
  const focused: string[] = [];
  const keys = { nav, focusedKey: 'r49', pageStep: navPageSteps(nav, 'r49', 100), blocked: false, isPinnedTooltip: () => false, onFocusKey: (key: string) => { focused.push(key); }, onActivate: () => {} };
  handleFamilyKeyDown(mkEvent('PageUp').event, keys);
  handleFamilyKeyDown(mkEvent('PageDown').event, { ...keys, focusedKey: 'r0', pageStep: navPageSteps(nav, 'r0', 100) });
  assert.deepEqual(focused, ['r45', 'r4']);
});

test('rv10 R2-S6: a mirror the lane withholds clears the installed rows once, and only a new field or selection repopulates', () => {
  const m1 = { rows: [], total: 0, first: 0 } as unknown as MirrorModel;
  const m2 = { rows: [], total: 0, first: 0 } as unknown as MirrorModel;
  assert.equal(mirrorToSync(m1, null, 'A', null), m1, 'the first mirror is shown');
  assert.equal(mirrorToSync(m1, m1, 'A', 'A'), undefined, 'the same field and selection do nothing, however many frames install');
  assert.equal(mirrorToSync(undefined, m1, 'A', 'A'), null, 'a withheld mirror clears the stale rows once');
  assert.equal(mirrorToSync(undefined, null, 'A', 'A'), undefined, 'and not again');
  assert.equal(mirrorToSync(m2, null, 'A', 'A'), m2, 'the current field repopulates');
  assert.equal(mirrorToSync(m2, m2, 'B', 'A'), m2, 'a new selection re-syncs the same rows');
});

test('rv10 R2-S5: rebuilt projection options equal in structure keep the earlier object, so their revision survives a span-only context change', () => {
  const looks = new Map([['GER', 'Germany']]);
  const first = { kinds: ['war', 'peace'], selection: { tags: ['GER'], all: false }, looks, top: null };
  const rebuilt = { kinds: ['war', 'peace'], selection: { tags: ['GER'], all: false }, looks, top: null };
  assert.equal(keepEqual(first, rebuilt), first, 'same filters and selection after the span grew: the same options object');
  const changed = { ...rebuilt, selection: { tags: ['GER', 'ITA'], all: false } };
  assert.equal(keepEqual(first, changed), changed, 'a changed selection is a new options object');
  assert.notEqual(keepEqual(first, { ...rebuilt, looks: new Map(looks) }), first, 'a map is compared by reference, never walked');
  assert.equal(keepEqual(undefined, rebuilt), rebuilt);
});

test('rv18 S18: a changed source plan replays the current row window to the replacement pager', () => {
  const all: SourcePlan = { owner: 'all', entries: [] };
  const filtered: SourcePlan = { owner: 'filtered', entries: [] };
  const win = { first: 596, count: 18 };
  const first = nextReportedRowWindow(null, win, 200, all);
  assert.deepEqual(first, { window: win, hour: 200, plan: all });
  assert.equal(nextReportedRowWindow(first, win, 200, all), null, 'same plan, same window, same hour is deduped');
  assert.equal(nextReportedRowWindow(first, win, 205, all), null, 'same plan, same window while advancing forward is deduped');
  assert.deepEqual(nextReportedRowWindow(first, win, 195, all), { window: win, hour: 195, plan: all }, 'seeking back replays the window');
  assert.deepEqual(
    nextReportedRowWindow(first, win, 200, filtered),
    { window: win, hour: 200, plan: filtered },
    'changing the plan replays the unchanged window at the same hour',
  );
  assert.deepEqual(
    nextReportedRowWindow(first, win, 205, filtered),
    { window: win, hour: 205, plan: filtered },
    'changing the plan replays the unchanged window even when the installed hour moved forward',
  );
  assert.equal(nextReportedRowWindow(first, null, 205, filtered), null, 'no requested window reports nothing');
});

test('rv21 S2: a new installed head that pushes a gap below a stationary viewport reports the updated data window past the head', () => {
  const source = { id: 'log-src', revision: 'r1', path: 'p', metadata: { branches: [{ id: '1', firstHour: 0, lastHour: 5000 }] } } as unknown as DashboardSource;
  const HEAD = 256;
  const entry = pagedTailEntry('main', source, '1', 0, { product: 'event-choices-tail', branchId: '1', facets: [], from: null, limit: HEAD }, true);
  const plan: SourcePlan = { owner: 'log', entries: [entry] };
  const columns = [{ key: 'event', label: 'Event', as: 'plain', offered: ['plain'], priority: 0 }] as never;
  const settings = tableSettingsOf(undefined, columns, { density: 'normal', sort: { column: 'event', direction: 'descending' } });
  const rowHeight = ROW_HEIGHT.normal;
  const viewport = { width: 600, height: 6 * rowHeight, dpr: 1, scrollLeft: 0, scrollTop: 247 * rowHeight, zoomWindow: null };

  const fieldsBefore = { table: { count: 301, totalHeight: 301 * rowHeight, gapBefore: [255] }, meta: {} };
  const firstUpdate = reconcileInstalledRowWindow(null, null, tableHostAdapter, plan, viewport, settings, fieldsBefore, 100);
  assert.deepEqual(firstUpdate.requested, { first: 243, count: 14, dataFirst: 243, dataCount: 13 });
  assert.ok(firstUpdate.report);

  const pager = new SnapshotPager(HEAD, DEFAULT_TAIL_PAGE_SIZE);
  const initialStep = pager.window(firstUpdate.report.window, firstUpdate.report.hour);
  assert.deepEqual(initialStep.fetch, [], 'with the gap inside display rows 243..256, data ordinals 243..255 stay inside the 256-row head');

  const fieldsAfter = { table: { count: 303, totalHeight: 303 * rowHeight, gapBefore: [257] }, meta: {} };
  const nextUpdate = reconcileInstalledRowWindow(
    firstUpdate.report,
    firstUpdate.requested,
    tableHostAdapter,
    plan,
    viewport,
    settings,
    fieldsAfter,
    105,
  );
  assert.equal(nextUpdate.changed, true, 'gap pushed below viewport changes dataCount from 13 to 14');
  assert.deepEqual(nextUpdate.requested, { first: 243, count: 14, dataFirst: 243, dataCount: 14 });
  assert.deepEqual(nextUpdate.report, { window: { first: 243, count: 14, dataFirst: 243, dataCount: 14 }, hour: 105, plan });

  const afterStep = pager.window(nextUpdate.report!.window, nextUpdate.report!.hour);
  assert.deepEqual(
    afterStep.fetch,
    [
      { offset: 0, limit: DEFAULT_TAIL_PAGE_SIZE, hour: 105 },
      { offset: 256, limit: DEFAULT_TAIL_PAGE_SIZE, hour: 105 },
    ],
    'pager now snapshots the window crossing past the 256-row head, including data ordinal 256',
  );

  const quietUpdate = reconcileInstalledRowWindow(
    nextUpdate.report,
    nextUpdate.requested,
    tableHostAdapter,
    plan,
    viewport,
    settings,
    fieldsAfter,
    106,
  );
  assert.equal(quietUpdate.changed, false);
  assert.equal(quietUpdate.report, null, 'unchanged installed window on a forward step stays deduplicated');
});

test('rv19 S2: a selected Range reaches source planning and plan signature for cumulative Lines declarations', () => {
  const track = {
    id: 't1',
    label: 'Campaign',
    source: { id: 's1', revision: 'r1' },
    branch: { id: '1', firstHour: 100, lastHour: 600, gaps: [] },
    axisOffset: 0,
  } as unknown as BoundTrack;
  const countries = {
    all: false,
    tags: [countryKey(7)],
    holders: null,
    every: [countryKey(7)],
    top: null,
    membership: null,
    membershipRuns: null,
    unobserved: [],
    filter: { passing: null, unrecorded: [], bounds: [] },
    factions: null,
    factionUnobserved: [],
    factionIncomplete: [],
    rankedAt: () => [countryKey(7)],
    reason: null,
    known: () => true,
    unobservedIn: () => false,
  };
  const makeBaseCtx = (time: WidgetSpec['time']) => ({
    track,
    branch: track.branch,
    lastRecordedHour: 600,
    widget: {
      id: 'w1',
      kind: 'air-kills-losses',
      view: 'lines',
      trackIds: ['t1'],
      time,
      countries: { kind: 'selected', countries: [countryKey(7)] },
      options: { measure: 'lost' },
    } as unknown as WidgetSpec,
    options: { measure: 'lost' },
    following: time.mode === 'follow',
    aux: undefined,
    countryTag: null,
    countrySubject: null,
    countryName: '',
    countries,
    gateSpan: (a: number, b: number) => ({ firstHour: a, lastHour: b } as never),
  });

  const rangeA = planFamilySources(
    makeBaseCtx({ mode: 'range', firstOffset: 120, lastOffset: 240 }),
    { firstHour: 220, lastHour: 340 },
    true,
    airKillsLossesLinesDeclaration.sources,
    { following: false },
  );
  assert.deepEqual(rangeA.ctx.staticSpan, [220, 340]);
  const dealtA = rangeA.plan?.entries.find(e => e.name === 'dealt')?.read;
  const takenA = rangeA.plan?.entries.find(e => e.name === 'taken')?.read;
  const headA = rangeA.plan?.entries.find(e => e.name === 'head')?.read;
  const hoverA = rangeA.plan?.entries.find(e => e.name === 'hover')?.read;
  assert.equal(dealtA?.shape === 'pixel-binned' ? dealtA.family.fromHour : undefined, 220);
  assert.equal(takenA?.shape === 'pixel-binned' ? takenA.family.fromHour : undefined, 220);
  assert.equal(headA?.shape === 'point' ? headA.params?.fromHour : undefined, 220);
  assert.equal(hoverA?.shape === 'exact-hover' ? hoverA.params?.fromHour : undefined, 220);

  const rangeB = planFamilySources(
    makeBaseCtx({ mode: 'range', firstOffset: 180, lastOffset: 240 }),
    { firstHour: 280, lastHour: 340 },
    true,
    airKillsLossesLinesDeclaration.sources,
    { following: false },
  );
  assert.deepEqual(rangeB.ctx.staticSpan, [280, 340]);
  const headB = rangeB.plan?.entries.find(e => e.name === 'head')?.read;
  assert.equal(headB?.shape === 'point' ? headB.params?.fromHour : undefined, 280);
  assert.notEqual(rangeA.sig, rangeB.sig, 'changing the selected range changes the plan signature');

  const unchangedOwnerBuilder = (ctx: Parameters<typeof airKillsLossesLinesDeclaration.sources>[0]) => ({
    owner: `fixed:${ctx.branch.id}`,
    entries: [],
  });
  const sig1 = planFamilySources(makeBaseCtx({ mode: 'range', firstOffset: 120, lastOffset: 240 }), { firstHour: 220, lastHour: 340 }, true, unchangedOwnerBuilder, { following: false }).sig;
  const sig2 = planFamilySources(makeBaseCtx({ mode: 'range', firstOffset: 180, lastOffset: 240 }), { firstHour: 280, lastHour: 340 }, true, unchangedOwnerBuilder, { following: false }).sig;
  assert.notEqual(sig1, sig2, 'the resolved range is part of the plan signature even when owner ignores it');

  const followPlan = planFamilySources(
    makeBaseCtx({ mode: 'follow' }),
    null,
    true,
    airKillsLossesLinesDeclaration.sources,
    { following: true },
  );
  assert.equal(followPlan.ctx.staticSpan, null);
  const followHead = followPlan.plan?.entries.find(e => e.name === 'head')?.read;
  assert.equal(followHead?.shape === 'point' ? followHead.params?.fromHour : 'missing', undefined);
});

test('#849: header summary folds notes and partial notices into one compact marker, Details diagnostics, and accessible description', () => {
  assert.equal(resolveHeaderSummary({}), null);

  const plain = resolveHeaderSummary({ summaryText: 'Infantry Equipment · per day' });
  assert.deepEqual(plain, {
    label: 'Infantry Equipment · per day',
    tooltip: null,
    spoken: 'Infantry Equipment · per day',
    hasMarker: false,
  });
  assert.equal(mergeAccessibleDescription(null, plain), null);

  const noteOnly = resolveHeaderSummary({ notes: ['Loss totals include corrections.'] });
  assert.deepEqual(noteOnly, {
    label: 'Note',
    tooltip: 'Loss totals include corrections.',
    spoken: 'Note. Loss totals include corrections.',
    hasMarker: true,
  });
  assert.equal(mergeAccessibleDescription(null, noteOnly), 'Note. Loss totals include corrections.');

  const partialOnly = resolveHeaderSummary({ summaryPartial: 'Some rows missing.' });
  assert.deepEqual(partialOnly, {
    label: 'Partial',
    tooltip: 'Some rows missing.',
    spoken: 'Partial. Some rows missing.',
    hasMarker: true,
  });

  const combined = resolveHeaderSummary({
    summaryText: '69 completed',
    notes: ['Head: 5 in progress · Radio 50%', 'Head: 5 in progress · Radio 50%'],
  });
  assert.deepEqual(combined, {
    label: '69 completed',
    tooltip: 'Head: 5 in progress · Radio 50%',
    spoken: '69 completed. Head: 5 in progress · Radio 50%',
    hasMarker: true,
  });

  assert.deepEqual(
    mergeDiagnosticNotes(['Extra warning'], ['Shared note'], ['Shared note', 'Loss totals include corrections.']),
    ['Extra warning', 'Shared note', 'Loss totals include corrections.'],
  );
  assert.equal(
    mergeAccessibleDescription('69 completed. Head: 5 in progress · Radio 50%', combined),
    '69 completed. Head: 5 in progress · Radio 50%',
  );
});

test('the host identity snapshot takes a flag drawn after a paused seek although the lane asks no names', async () => {
  const never = <T,>() => new Promise<T>(() => {});
  const transport: IdentityTransport = {
    loadFacts: never,
    loadIdentities: never,
    loadGroups: never,
    loadStyles: never,
    loadRegions: never,
    loadPlaceNameRuns: never,
    loadLeaderNameRuns: never,
    loadFlags: async (_scope, size, batch) => ({
      size,
      flags: batch.map((choice, flag) => ({ flag, asset: `${choice.keys[0]}.tga`, key: `${choice.keys[0]}:${size}`, width: 41, height: 26, png: `flag://${choice.keys[0]}` })),
      unavailable: [],
      warnings: [],
    }),
  };
  const broker = new IdentityBroker({ transport });
  const scope: IdentityScope = { sourceId: 's1', revision: 'r1', branchId: '1', content: 1 };
  const store = identitySnapshots(broker, scope, undefined, [0, 1000]);
  let changes = 0;
  const stop = store.subscribe(() => { changes++; });
  const before = store.read();
  assert.equal(before?.drawnFlags, undefined, 'no flag is drawn before the lane asks');

  requestDemandedFlags(broker, scope, [flagArtHandle(flagChoiceKey({ keys: ['GER', null, null], reasons: [null, null, null] }))]);
  for (let turn = 0; turn < 50; turn++) await Promise.resolve();

  assert.ok(changes > 0, 'the snapshot store hears the flag land');
  const after = store.read();
  assert.notEqual(after, before, 'a new snapshot is handed to the lane');
  assert.deepEqual(Object.values(after?.drawnFlags ?? {}).map(flag => flag.url), ['flag://GER']);
  assert.equal(store.read(), after, 'reading again without a change keeps the same snapshot');
  stop();
  broker.dispose();
});

test('every member so far masks each member from its first membership, follows its country through a tag change, and never a reused tag', () => {
  const identity = (firstHour: number, lastHour: number, slot: number, countryId: number) => ({ firstHour, lastHour, slot, countryId });
  const groups: CountryGroupReply = {
    branchId: '1',
    humans: [],
    factions: [{
      id: '7', name: 'Axis', nameReason: null, names: [], recordedName: 'AXIS', firstHour: 50, lastHour: 400,
      members: [{ tag: 'GER', firstHour: 50, lastHour: 400 }, { tag: 'CCC', firstHour: 100, lastHour: 200 }, { tag: 'AAA', firstHour: 200, lastHour: 260 }],
      leaders: [{ tag: 'GER', firstHour: 50, lastHour: 400 }],
      incomplete: [],
    }],
    wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [
      { tag: 'GER', identities: [identity(0, 1000, 1, 101)] },
      { tag: 'AAA', identities: [identity(0, 299, 7, 707), identity(500, 1000, 8, 808)] },
      { tag: 'BBB', identities: [identity(300, 1000, 7, 707)] },
      { tag: 'CCC', identities: [identity(0, 149, 9, 909), identity(150, 1000, 10, 1010)] },
    ],
    unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    warnings: [],
  };
  const selection = { kind: 'group' as const, group: { type: 'faction' as const, id: '7' }, membership: 'ever' as const, aggregate: false };
  const masked = resolveCountries(selection, [], groups, 1000, Number.MAX_SAFE_INTEGER);
  assert.deepEqual([...masked.membership!.entries()], [
    ['GER', [{ tag: 'GER', firstHour: 50, lastHour: 1000 }]],
    ['CCC', [{ tag: 'CCC', firstHour: 100, lastHour: 1000 }]],
    ['AAA', [{ tag: 'AAA', firstHour: 200, lastHour: 299 }]],
    ['BBB', [{ tag: 'BBB', firstHour: 300, lastHour: 1000 }]],
  ], 'each member country stays one from its first membership to the branch end, under every tag it holds; both holders of one merged run are members; the later holder of an old member tag is not');
  assert.deepEqual(rankedCandidates(masked, 75, masked.tags), ['GER'], 'at hour 75 only the founder is a member so far');
  assert.deepEqual(rankedCandidates(masked, 280, masked.tags), ['GER', 'CCC', 'AAA']);
  assert.deepEqual(rankedCandidates(masked, 600, masked.tags), ['GER', 'CCC', 'BBB'], 'after leaving, the second holder of CCC and the renamed AAA are still members so far; the new holder of AAA is not');

  const atHour = { ...selection, membership: 'hour' as const };
  const hourly = resolveCountries(atHour, [], groups, 1000, Number.MAX_SAFE_INTEGER);
  assert.deepEqual([...hourly.membership!.entries()], [
    ['GER', [{ tag: 'GER', firstHour: 50, lastHour: 400 }]],
    ['CCC', [{ tag: 'CCC', firstHour: 100, lastHour: 200 }]],
    ['AAA', [{ tag: 'AAA', firstHour: 200, lastHour: 260 }]],
  ], 'membership at the hour keeps its recorded runs');
});

const everFaction = (unknown: { firstHour: number; lastHour: number }[], members = [{ tag: 'GER', firstHour: 0, lastHour: 1000 }, { tag: 'ITA', firstHour: 600, lastHour: 1000 }]): CountryGroupReply => ({
  branchId: '1',
  humans: [],
  factions: [{ id: '42', name: null, nameReason: null, names: [], recordedName: '', firstHour: 0, lastHour: 1000, members, leaders: [], incomplete: unknown }],
  wars: [], warStarts: [], capitulations: [], identityChanges: [],
  countryIdentities: [
    { tag: 'GER', identities: [{ firstHour: 0, lastHour: 1000, slot: 1, countryId: 101 }] },
    { tag: 'ITA', identities: [{ firstHour: 0, lastHour: 1000, slot: 2, countryId: 102 }] },
  ],
  unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
  warnings: [],
} as unknown as CountryGroupReply);
const everOf = (group: { type: 'faction'; id: string } | { type: 'war'; tag: string }) => ({ kind: 'group' as const, group, membership: 'ever' as const, aggregate: false });

test('every member so far is dated for filters: a later member never passes before it joins, and nothing passes from the first unknown roster hour (#1016)', () => {
  const groups = everFaction([{ firstHour: 900, lastHour: 950 }]);
  const selection = everOf({ type: 'faction', id: '42' });
  const match = countryMatch(selection, [], groups, 1000);
  assert.equal(match.reason, null, 'a roster gap no longer refuses the whole campaign');
  assert.deepEqual([match.matches('GER', 200), match.matches('ITA', 200)], [true, false], 'Italy, joining at 600, does not pass at 200');
  assert.deepEqual([match.matches('GER', 899), match.matches('ITA', 899)], [true, true]);
  assert.deepEqual([match.matches('GER', 900), match.matches('ITA', 960)], [false, false], 'from the first unknown roster hour nothing passes, even after the gap ends');
  assert.deepEqual([membershipUnrecordedAt(selection, groups, 899), membershipUnrecordedAt(selection, groups, 900), membershipUnrecordedAt(selection, groups, 960)], [null, EVER_UNRECORDED, EVER_UNRECORDED]);
  const noEnemies = { ...everFaction([]), wars: [], unobserved: { humans: [], factions: [], wars: [{ firstHour: 20, lastHour: 30 }], unattributed: [] } } as unknown as CountryGroupReply;
  assert.equal(countryMatch({ ...everOf({ type: 'war', tag: 'GER' }), membership: 'hour' }, [], noEnemies, 1000).reason, 'Group membership is incomplete for this campaign.', 'membership at the hour keeps its campaign-wide reason, never a date-specific one');
});

test('every member so far is dated for multi-country views: each mask ends before the first unknown roster hour, and the hours from it are unobserved (#1016)', () => {
  const groups = everFaction([{ firstHour: 300, lastHour: 320 }]);
  const selection = everOf({ type: 'faction', id: '42' });
  const resolved = resolveCountries(selection, [], groups, 1000, Number.MAX_SAFE_INTEGER);
  assert.equal(resolved.reason, null);
  assert.deepEqual([...resolved.membership!.entries()], [['GER', [{ tag: 'GER', firstHour: 0, lastHour: 299 }]]], 'Germany is a member so far up to 299; Italy, joining after the gap, is never presented');
  assert.deepEqual(effectiveCountryUnobserved(selection, resolved.unobserved), [[300, Number.MAX_SAFE_INTEGER]], 'members so far are unknown from 300 on, never the roster of 299 presented as current');
  const renamed = { ...everFaction([], [{ tag: 'OLD', firstHour: 0, lastHour: 9 }, { tag: 'NEW', firstHour: 10, lastHour: 1000 }]),
    countryIdentities: [{ tag: 'OLD', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 101 }] }, { tag: 'NEW', identities: [{ firstHour: 10, lastHour: 1000, slot: 1, countryId: 101 }] }] } as unknown as CountryGroupReply;
  const one = resolveCountries(selection, [], renamed, 1000, 1);
  assert.deepEqual([one.tags, one.dropped], [['OLD', 'NEW'], 0], 'a limit counts countries, never a renamed country twice');
  const swapped = { ...everFaction([], [{ tag: 'AAA', firstHour: 0, lastHour: 9 }, { tag: 'BBB', firstHour: 10, lastHour: 1000 }, { tag: 'AAA', firstHour: 10, lastHour: 1000 }]),
    countryIdentities: [{ tag: 'AAA', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 101 }, { firstHour: 10, lastHour: 1000, slot: 2, countryId: 102 }] },
      { tag: 'BBB', identities: [{ firstHour: 10, lastHour: 1000, slot: 1, countryId: 101 }] }] } as unknown as CountryGroupReply;
  const first = resolveCountries(selection, [], swapped, 1000, 1);
  assert.deepEqual([first.dropped, [...first.membership!.entries()]], [1, [['AAA', [{ tag: 'AAA', firstHour: 0, lastHour: 9 }]], ['BBB', [{ tag: 'BBB', firstHour: 10, lastHour: 1000 }]]]],
    'the limit keeps the first country, and the second country that took its old tag is dropped');
  const together = { ...everFaction([], [{ tag: 'ZZZ', firstHour: 0, lastHour: 1000 }, { tag: 'AAA', firstHour: 0, lastHour: 1000 }]),
    countryIdentities: [{ tag: 'AAA', identities: [{ firstHour: 0, lastHour: 1000, slot: 7, countryId: 107 }] }, { tag: 'ZZZ', identities: [{ firstHour: 0, lastHour: 1000, slot: 1, countryId: 101 }] }] } as unknown as CountryGroupReply;
  assert.deepEqual(resolveCountries(selection, [], together, 1000, 1).tags, ['AAA'], 'countries joining together are kept in tag order, as membership at the hour keeps them');
  const left = { ...everFaction([], [{ tag: 'AAA', firstHour: 0, lastHour: 5 }]),
    countryIdentities: [{ tag: 'AAA', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 101 }] },
      { tag: 'BBB', identities: [{ firstHour: 0, lastHour: 9, slot: 2, countryId: 102 }, { firstHour: 10, lastHour: 1000, slot: 1, countryId: 101 }] }] } as unknown as CountryGroupReply;
  const so = resolveCountries(selection, [], left, 1000, Number.MAX_SAFE_INTEGER);
  assert.deepEqual([tagsAt(so, 3), tagsAt(so, 12)], [['AAA'], ['BBB']], 'a choice at an hour offers the member under its tag then, never another holder of its later tag');
  const unread = resolveCountries({ ...selection, membership: 'hour' }, [], everFaction([{ firstHour: 10, lastHour: 10 }], [{ tag: 'AAA', firstHour: 9, lastHour: 11 }]), 1000, Number.MAX_SAFE_INTEGER);
  assert.deepEqual([tagsAt(unread, 9), tagsAt(unread, 10), tagsAt(unread, 11)], [['AAA'], [], ['AAA']], 'an hour with an unread roster offers no member');
  assert.deepEqual([everStateAt(everBounds(selection, groups)!, 299).kind, everStateAt(everBounds(selection, groups)!, 300).kind], [null, 'unrecorded']);
});

test('an empty group with a later roster gap is known and empty before the gap and not recorded from it, never refused for the campaign (#1016)', () => {
  const groups = { ...everFaction([]), wars: [], unobserved: { humans: [], factions: [], wars: [{ firstHour: 20, lastHour: 30 }], unattributed: [] } } as unknown as CountryGroupReply;
  const selection = everOf({ type: 'war', tag: 'GER' });
  const resolved = resolveCountries(selection, [], groups, 1000, Number.MAX_SAFE_INTEGER);
  assert.deepEqual([resolved.tags, resolved.reason], [[], null]);
  const bounds = everBounds(selection, groups)!;
  assert.deepEqual([everStateAt(bounds, 10).kind, everStateAt(bounds, 19).kind, everStateAt(bounds, 20).kind, everStateAt(bounds, 500).kind], ['empty', 'empty', 'unrecorded', 'unrecorded']);
  assert.deepEqual([everStateAt(bounds, 10).until, everStateAt(bounds, 20).from], [19, 20], 'the state changes only at the boundary hour');
  assert.equal(worldFilter(selection, [], groups, 10).reason, NO_MEMBERS.yet);
  assert.deepEqual(resolveCountries(selection, [], everFaction([], []), 1000, Number.MAX_SAFE_INTEGER).reason, NO_MEMBERS.ever, 'with no gap, a group with no members is empty for the whole campaign');
});

test('the dated rule answers as the recording resolver members_at does on its own fixture (a_group_is_its_members_at_the_asked_hour_and_unknown_membership_stays_unknown)', () => {
  const groups = everFaction([{ firstHour: 300, lastHour: 320 }]);
  const selection = everOf({ type: 'faction', id: '42' });
  const runs = groupRuns(groups, selection.group)!;
  const unknown = groupUnobserved(groups, selection.group);
  const members = (hour: number) => groupMembers(runs, 'ever', hour, Infinity, unknown);
  assert.deepEqual(members(0), ['GER'], 'the first-membership hour');
  assert.deepEqual(members(200), ['GER'], 'every member so far, never one from later');
  assert.deepEqual(members(299), ['GER'], 'the last hour before the unknown roster');
  assert.equal(members(300), null, 'the first unknown roster hour');
  assert.equal(members(700), null, 'a past unknown roster leaves who was ever a member unknown');
  assert.equal(worldFilter(selection, [], groups, 700).reason, EVER_UNRECORDED);
});

test('a single-country widget that names its subject itself opens on branch coverage, not on a live country (#873 S14)', () => {
  const policy = factionPooledResourcesDeclaration.countries;
  assert.equal(gatedByCountry(policy, {}), true, 'with no faction named, the chosen country decides');
  assert.equal(gatedByCountry(policy, { faction: '' }), true);
  assert.equal(gatedByCountry(policy, { faction: 'axis' }), false, 'an explicit faction needs no live country');
  assert.equal(gatedByCountry({ mode: 'single' }, { faction: 'axis' }), true, 'a widget without a subject option keeps its country gate');
  assert.equal(gatedByCountry({ mode: 'multi' }, {}), false);
});

test('a war group chosen by recorded slot matches that slot under every tag it held, and not the later holder of its old tag (#932 R2-S15)', () => {
  const reply = {
    wars: [
      { first: 'AAA', second: 'POL', firstHour: 10, lastHour: 60 },
      { first: 'BBB', second: 'FRA', firstHour: 40, lastHour: 90 },
      { first: 'AAA', second: 'ITA', firstHour: 70, lastHour: 80 },
    ],
    countryIdentities: [
      { tag: 'AAA', identities: [{ slot: 7, countryId: 707, firstHour: 0, lastHour: 49 }, { slot: 8, countryId: 808, firstHour: 65, lastHour: 100 }] },
      { tag: 'BBB', identities: [{ slot: 7, countryId: 707, firstHour: 50, lastHour: 100 }] },
    ],
  } as unknown as CountryGroupReply;
  assert.deepEqual(groupRuns(reply, { type: 'war', tag: countryKey(7) }), [
    { tag: 'POL', firstHour: 10, lastHour: 49 },
    { tag: 'FRA', firstHour: 50, lastHour: 90 },
  ], 'slot 7 is at war with Poland while it is AAA and with France once it is BBB; the Italian war is slot 8, the later AAA');
  assert.deepEqual(groupRuns(reply, { type: 'war', tag: 'AAA' }), [
    { tag: 'POL', firstHour: 10, lastHour: 60 },
    { tag: 'ITA', firstHour: 70, lastHour: 80 },
  ], 'a war group chosen by tag keeps matching the tag');
});

test('the hovered row is the subject the row would activate, reported when it changes, never while the pointer rests there or playback advances under it, and cleared on leaving', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  type Listener = (event: unknown) => void;
  const listeners = new Map<string, Set<Listener>>();
  const body = {
    addEventListener(type: string, fn: Listener) { (listeners.get(type) ?? listeners.set(type, new Set()).get(type)!).add(fn); },
    removeEventListener(type: string, fn: Listener) { listeners.get(type)?.delete(fn); },
    dispatch(type: string, event: Record<string, unknown>) { for (const fn of listeners.get(type) ?? []) fn({ target: body, preventDefault() {}, ...event }); },
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 200, height: 100 }),
    contains: (node: unknown) => node === body,
  };
  const nav: FamilyNavTuple[] = [['GER', 'Germany', 0, -1, '', null], ['FRA', 'France', 1, -1, '', null]];
  const picture = (id: number): InstalledPicture => {
    const installed: InstalledPicture = {
      id, owner: 'owner-A', generation: { seek: 1, reset: 0, binding: 1 }, factRevisions: {}, validInterval: [100, 120], through: 100, hour: displayedHour(100 + id),
      viewport: { width: 200, height: 100, dpr: 1, scrollLeft: 0, scrollTop: 0 }, optionsRevision: '', themeRevision: '',
      actionKeys: lazyActionKeys(nav, displayedHour(100 + id)), accessibleDescription: null,
    };
    recordInstalled(installed, { nav }, null);
    return installed;
  };
  let installed = picture(7);
  const pending: ((key: string | null, meta?: PointerHitMeta) => void)[] = [];
  const fakePipeline = {
    pointer(_pipe: number, _slot: number, _pt: unknown, cb?: (key: string | null, meta?: PointerHitMeta) => void) { if (cb) pending.push(cb); },
    inspect() {},
    view() { return null; },
  } as unknown as Pipeline;
  const reply = (key: string | null) => { while (pending.length) pending.shift()!(key, { installedId: installed.id, owner: installed.owner }); };
  const rows: (string | null)[] = [];
  let hoverNotices = 0;
  const publish = rowHoverPublisher(() => tableHostAdapter, () => installedRecordOf(installed), () => subject => rows.push(subject));
  const stop = bindFamilyPointer({
    bodyNode: body as unknown as HTMLElement, pictureNode: body as unknown as HTMLElement, menuHost: body as unknown as HTMLElement,
    pipeline: fakePipeline, pipe: 1, tooltips: new FamilyTooltipController('delayed-worker'), inspectedKeyRef: { current: null }, inspectedHitRef: { current: null },
    lastPointerRef: { current: null }, focusInsideRef: { current: false }, focusedKeyRef: { current: null }, focusAnchorRef: { current: { x: 0, y: 0 } },
    accessibleRef: { current: body as unknown as HTMLElement }, isBlocked: () => false, getInstalled: () => installed, getViewportHeight: () => 800,
    parseHit: parseTableHit,
    onHoverChange: () => { hoverNotices++; },
    onHoverHit: publish,
    onActivate: () => {},
  });
  body.dispatch('pointermove', { isTrusted: true, clientX: 20, clientY: 10 });
  reply('row:GER');
  for (const id of [8, 9, 10]) {
    installed = picture(id);
    body.dispatch('pointermove', { isTrusted: true, clientX: 10 + id, clientY: 10 });
    reply('row:GER');
  }
  assert.deepEqual(rows, ['GER'], 'one report while the pointer rests on GER and playback advances under it');
  assert.equal(hoverNotices, 1, 'a new picture under the same row is no hover change for the hour publisher');
  body.dispatch('pointermove', { isTrusted: true, clientX: 21, clientY: 30 });
  reply('row:FRA');
  assert.deepEqual(rows, ['GER', 'FRA'], 'a new row under the pointer is reported once');
  body.dispatch('pointerleave', { isTrusted: true, clientX: 300, clientY: 300 });
  t.mock.timers.tick(HOVER_CLOSE_MS);
  assert.deepEqual(rows, ['GER', 'FRA', null], 'leaving clears the hovered row');
  body.dispatch('pointermove', { isTrusted: true, clientX: 20, clientY: 10 });
  body.dispatch('pointerleave', { isTrusted: true, clientX: 300, clientY: 300 });
  t.mock.timers.tick(HOVER_CLOSE_MS);
  reply('row:GER');
  assert.deepEqual(rows, ['GER', 'FRA', null], 'a hit answered after the pointer left reports no row');
  body.dispatch('pointermove', { isTrusted: true, clientX: 22, clientY: 10 });
  body.dispatch('pointerleave', { isTrusted: true, clientX: 300, clientY: 300 });
  t.mock.timers.tick(HOVER_CLOSE_MS);
  body.dispatch('pointermove', { isTrusted: true, clientX: 22, clientY: 30 });
  const meta = { installedId: installed.id, owner: installed.owner };
  pending.shift()!('row:GER', meta);
  pending.shift()!('row:FRA', meta);
  assert.deepEqual(rows, ['GER', 'FRA', null, 'FRA'], 'a hit from before the tooltip closed is not reported on the next visit');
  stop();

  const lane = (id: number, row: string) => {
    const shown = picture(id);
    recordInstalled(shown, { extent: { rows: [{ id: row, label: row }] } }, null);
    return shown;
  };
  installed = lane(20, 'GER');
  const laneRows: (string | null)[] = [];
  const stopLanes = bindFamilyPointer({
    bodyNode: body as unknown as HTMLElement, pictureNode: body as unknown as HTMLElement, menuHost: body as unknown as HTMLElement,
    pipeline: fakePipeline, pipe: 1, tooltips: new FamilyTooltipController('delayed-worker'), inspectedKeyRef: { current: null }, inspectedHitRef: { current: null },
    lastPointerRef: { current: null }, focusInsideRef: { current: false }, focusedKeyRef: { current: null }, focusAnchorRef: { current: { x: 0, y: 0 } },
    accessibleRef: { current: body as unknown as HTMLElement }, isBlocked: () => false, getInstalled: () => installed, getViewportHeight: () => 800,
    parseHit: parseHeatLanesHit,
    onHoverHit: rowHoverPublisher(() => heatLanesHostAdapter, () => installedRecordOf(installed), () => subject => laneRows.push(subject)),
    onActivate: () => {},
  });
  for (const [x, hit] of [[40, 'lane:#0@100'], [41, 'lane:#0@101'], [42, 'lane:#0']] as const) {
    body.dispatch('pointermove', { isTrusted: true, clientX: x, clientY: 10 });
    reply(hit);
  }
  assert.deepEqual(laneRows, ['GER'], 'moving along one heat lane across hours is one row');
  installed = lane(21, 'FRA');
  body.dispatch('pointermove', { isTrusted: true, clientX: 43, clientY: 10 });
  reply('lane:#0');
  assert.deepEqual(laneRows, ['GER', 'FRA'], 'the same lane key over another lane in a new picture is a new row');
  body.dispatch('pointermove', { isTrusted: true, clientX: 44, clientY: 30 });
  stopLanes();
  reply('lane:#0');
  assert.deepEqual(laneRows, ['GER', 'FRA', null], 'a hit answered after the binding is gone reports no row');
});

test('R4-S3 host: ordinal bounds, gap skipping, key directions and held mirror focus', () => {
  const extents = rowExtents(36, [[2, 72], [5, 54]])!;
  assert.deepEqual(tableOrdinalBounds({ count: 10, totalHeight: 414, extents }, [], 0), { top: 0, height: 36 });
  assert.deepEqual(tableOrdinalBounds({ count: 10, totalHeight: 414, extents }, [], 2), { top: 72, height: 72 });
  assert.deepEqual(tableOrdinalBounds({ count: 10, totalHeight: 414, extents }, [], 6), { top: 270, height: 36 });
  const navSample: FamilyNavTuple[] = [['R610', 'Row 610', 610, -1, '', null, 610 * 36, 36]];
  assert.deepEqual(tableOrdinalBounds({ count: 2000, totalHeight: 2000 * 36 + 44 }, navSample, 1999), { top: 1999 * 36, height: 36 }, 'uses row height from nav even when an end note pads totalHeight');
  assert.deepEqual(tableOrdinalBounds({ count: 2000, totalHeight: 2000 * 36 }, [], 1999), { top: 1999 * 36, height: 36 }, 'falls back to average height when nav is empty while loading');
  assert.equal(tableOrdinalBounds({ count: 10, totalHeight: 360 }, [], 10), null);

  assert.equal(nearestDataOrdinal(0, 10, undefined, 1), 0);
  assert.equal(nearestDataOrdinal(0, 10, [0], 1), 1, 'Home skips a leading gap row');
  assert.equal(nearestDataOrdinal(9, 10, [9], -1), 8, 'End skips a trailing gap row');
  assert.equal(nearestDataOrdinal(5, 10, [5], 1), 6, 'forward move skips a gap row');
  assert.equal(nearestDataOrdinal(5, 10, [5], -1), 4, 'backward move skips a gap row');
  assert.equal(nearestDataOrdinal(0, 1, [0], 1), null, 'a log holding only a gap has no data ordinal');

  const moves: (readonly [number, number | undefined])[] = [];
  const keyOpts = {
    nav: navSample,
    totalCount: 2000,
    focusedKey: 'R610',
    pageStep: { down: 5, up: 5 },
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: () => {},
    onFocusOrdinal: (ord: number, dir?: 1 | -1) => moves.push([ord, dir]),
    onActivate: () => {},
  };
  for (const key of ['Home', 'End', 'ArrowDown', 'ArrowUp', 'PageDown', 'PageUp']) {
    handleFamilyKeyDown(mkEvent(key).event, keyOpts);
  }
  assert.deepEqual(moves, [[0, 1], [1999, -1], [611, 1], [609, -1], [615, 1], [605, -1]]);

  const pageTable = { count: 2000, totalHeight: 2000 * 36 };
  const narrowSlice: FamilyNavTuple[] = Array.from({ length: 9 }, (_, i) => [`R${615 + i}`, `Row ${615 + i}`, 615 + i, -1, '', null, (615 + i) * 36, 36]);
  assert.deepEqual(tablePageSteps(pageTable, narrowSlice, 619, 20 * 36), { down: 19, up: 19 }, 'PageDown and PageUp cross past the four overscan rows using viewport height and absolute row geometry');
  assert.deepEqual(tablePageSteps(pageTable, [], 1999, 20 * 36), { down: 1, up: 19 }, 'PageUp from a pending End ordinal uses viewport height even when nav is empty');
  assert.deepEqual(tablePageSteps({ count: 10, totalHeight: 414, extents }, [], 0, 144), { down: 2, up: 1 }, 'variable extents account for taller rows in page steps');

  const foldedNav: FamilyNavTuple[] = Array.from({ length: 5 }, (_, i) => [`R${1990 + i}`, `Row ${1990 + i}`, 1990 + i, -1, '', null, (1990 + i) * 36, 36]);
  const foldedKeys = new Map(foldedNav.map(entry => [entry[0], true]));
  const endWindow = { first: 1990, count: 10 };
  const rowOf = (key: string, rowIndex: number, name = key) => ({ key, name, rowIndex, cells: [{ column: 'name', spoken: name }] });
  assert.equal(
    resolveInstalledOrdinal(navSample, 1999, pageTable, endWindow, { first: 610, total: 2000, rows: [rowOf('R610', 610)] }, new Map([['R610', true]])),
    null,
    'a stale slice before the endpoint window does not resolve End',
  );
  assert.equal(
    resolveInstalledOrdinal(foldedNav, 1999, pageTable, endWindow, { first: 1990, total: 2000, rows: [rowOf('R1990', 1990), rowOf('loading:1995', 1995, 'Loading')] }, foldedKeys),
    null,
    'an endpoint window that still holds loading rows keeps waiting',
  );
  assert.equal(
    resolveInstalledOrdinal(foldedNav, 1999, pageTable, endWindow, { first: 1990, total: 2000, rows: foldedNav.map(e => rowOf(e[0], e[2])) }, foldedKeys)?.[0],
    'R1994',
    'a settled endpoint window with a folded run resolves End onto its last navigable row',
  );

  const prevDoc = (globalThis as { document?: unknown }).document;
  const prevEl = (globalThis as { HTMLElement?: unknown }).HTMLElement;
  try {
    interface MockEl {
      textContent: string;
      tabIndex: number;
      dataset: Record<string, string>;
      attrs: Map<string, string>;
      children: MockEl[];
      parentElement: MockEl | null;
      readonly lastElementChild: MockEl | null;
      getAttribute(name: string): string | null;
      hasAttribute(name: string): boolean;
      setAttribute(name: string, value: string): void;
      removeAttribute(name: string): void;
      append(...nodes: MockEl[]): void;
      replaceChildren(...nodes: MockEl[]): void;
      contains(node: unknown): boolean;
      focus(): void;
      remove(): void;
    }
    const makeEl = (parentElement: MockEl | null = null): MockEl => {
      const el: MockEl = {
        textContent: '',
        tabIndex: -1,
        dataset: {},
        attrs: new Map(),
        children: [],
        parentElement,
        get lastElementChild() { return this.children[this.children.length - 1] ?? null; },
        getAttribute(name) { return this.attrs.get(name) ?? null; },
        hasAttribute(name) { return this.attrs.has(name); },
        setAttribute(name, value) { this.attrs.set(name, value); },
        removeAttribute(name) { this.attrs.delete(name); },
        append(...nodes) {
          for (const n of nodes) {
            const list = n.attrs.get('__frag') === '1' ? n.children : [n];
            for (const item of list) {
              item.parentElement = this;
              this.children.push(item);
              item.remove = () => {
                const i = this.children.indexOf(item);
                if (i >= 0) this.children.splice(i, 1);
              };
            }
          }
        },
        replaceChildren(...nodes) { this.children = []; this.append(...nodes); },
        contains(node) { return node === this || this.children.some(child => child.contains(node)); },
        focus() { (globalThis as unknown as { document: { activeElement: unknown } }).document.activeElement = el; },
        remove() {},
      };
      return el;
    };
    const doc = {
      createElement: () => makeEl(),
      createDocumentFragment: () => { const f = makeEl(); f.attrs.set('__frag', '1'); return f; },
      activeElement: null as unknown,
    };
    (globalThis as { document: unknown }).document = doc;
    (globalThis as { HTMLElement: unknown }).HTMLElement = Object;
    const table = makeEl();
    const tbody = makeEl(table);
    table.append(tbody);
    const focusedKeyRef = { current: 'R610' as string | null };
    const baseOpts = {
      selectedKey: null,
      rowsFocusable: true,
      focusedKeyRef,
      focusInsideRef: { current: true },
      restoringFocusRef: { current: false },
      lastMirrorKeyRef: { current: '' },
    };
    const row = (key: string, rowIndex: number) => ({ key, name: key, rowIndex, cells: [{ column: 'name', spoken: key }] });
    syncTableMirror(tbody as unknown as HTMLTableSectionElement, { first: 610, total: 2000, rows: [row('R610', 610)] }, baseOpts);
    tbody.children[0]!.focus();
    const movedWhileHolding = syncTableMirror(
      tbody as unknown as HTMLTableSectionElement,
      { first: 1990, total: 2000, rows: [row('loading:1990', 1990), row('loading:1999', 1999)] },
      { ...baseOpts, holdFocus: true },
    );
    assert.equal(movedWhileHolding, false, 'holding focus during a pending ordinal scroll does not trigger a fallback focus move');
    assert.equal(focusedKeyRef.current, 'R610', 'focusedKeyRef is preserved while waiting');
    assert.equal(doc.activeElement, table, 'DOM focus parks on the persistent table element while loading rows render');

    focusedKeyRef.current = 'R1999';
    const movedOnArrival = syncTableMirror(
      tbody as unknown as HTMLTableSectionElement,
      { first: 1990, total: 2000, rows: [row('R1990', 1990), row('R1999', 1999)] },
      { ...baseOpts, holdFocus: false },
    );
    assert.equal(movedOnArrival, false);
    assert.equal(doc.activeElement, tbody.children[1], 'once the target ordinal arrives, focus restores directly onto that row');
    assert.equal(tbody.children[1]!.getAttribute('aria-rowindex'), '2001');
  } finally {
    (globalThis as { document?: unknown }).document = prevDoc;
    (globalThis as { HTMLElement?: unknown }).HTMLElement = prevEl;
  }
});
