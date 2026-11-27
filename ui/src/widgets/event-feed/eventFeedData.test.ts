import assert from 'node:assert/strict';
import test from 'node:test';
import { fitTooltip, handleActiveTooltipKey, handleActiveTooltipWheel, hideTooltip, measureTooltipScroll, showTooltip, tooltipStore, tooltipText } from '../../charts/tooltip.ts';
import { tooltipRowOffsets, tooltipRowWindow } from '../../charts/tooltipScroll.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { datedIdentity } from '../../families/identity/dated.fixture.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import { buildColumnLayouts, selectVisibleColumns } from '../../families/table/layout.ts';
import { deriveRowTooltip, deriveTableAudit, deriveTableMirror, resolveTableRows, type TableRow } from '../../families/table/model.ts';
import { LOG_TABLE_BASE } from '../../families/table/logHelpers.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import { validateFeedRow } from './eventFeedData.ts';
import { eventFeedDeclaration, eventFeedProjection } from './eventFeedProjection.ts';
import type { EventDetailSubject } from '../event-choices/EventDetailWindow.tsx';
import type { EventFeedRow } from '../../generated/dashboard';

let serial = 0;
const row = (kind: EventFeedRow['kind'], hour: number, tags: string[], more: Partial<EventFeedRow> = {}): EventFeedRow => ({
  id: `native:1:${serial++}`, hour, readHour: hour, nameHour: hour, exact: true, category: 'warsPeace', kind, tags,
  participants: [tags[0] ?? null, tags[1] ?? null], item: null, amount: null, ...more });

test('a feed row is validated, an icon the dashboard cannot draw dropped, and malformed rows refused', () => {
  const event = row('warDeclared', 100, ['GER', 'POL'], { readHour: 101, cause: { how: 'a diplomatic action', notes: [], targetRecorded: false } });
  assert.deepEqual(validateFeedRow(event), event);
  const scripted = row('scriptEventFired', 100, ['GER', 'GER'], { category: 'politics',
    item: { token: 'news.1', name: 'News', icon: 'javascript:bad', description: null, options: [{ ordinal: 0, name: 'Fine' }] } });
  assert.equal(validateFeedRow(scripted).item?.icon, null);
  assert.throws(() => validateFeedRow({ ...event, participants: ['GER', 'ITA'] }), /malformed/,
    'roles disagree with the tags');
  assert.throws(() => validateFeedRow(null), /malformed/);
});

test('nuclear strike and decision state validation reject mismatched kinds', () => {
  const strike = { province: 13000, state: 1204, buildingLevels: 2.5, warSupport: -25, units: 1, manpowerLost: null, equipmentLost: null,
    notRecorded: ['equipment: writers not forwarded (1×)'] };
  const event = validateFeedRow({ ...row('nuclearStrike', 40, ['USA', 'JAP'], { readHour: 41, item: { token: 'nuclear_bomb', name: 'Atomic bomb', icon: null,
    description: null } }), strike });
  assert.equal(event.strike?.province, 13000);
  assert.throws(() => validateFeedRow({ ...row('warDeclared', 40, ['USA', 'JAP']), strike }), /malformed/, 'only a strike carries one');
  assert.throws(() => validateFeedRow(row('warDeclared', 100, ['GER', 'POL'], { decisionState: { kind: 'failed', observedHour: 100 } })), /malformed/);
});

test('event feed projection preserves selected option ordinal, keeps null recipient distinct from sender, restores nuclear strike effects, and projects server-folded lines', () => {
  const chosenRow = row('eventOptionSelected', 100, ['GER'], {
    category: 'politics',
    participants: ['GER', null],
    cause: { notes: [], selectedOption: 1, targetRecorded: false },
    item: { token: 'ev.1', name: 'Cabinet Crisis', icon: null, description: 'Body', options: [{ ordinal: 0, name: 'Stay' }, { ordinal: 1, name: 'Resign' }] },
  });
  const unknownRecipient = row('scriptEventFired', 100, ['GER'], {
    category: 'politics',
    participants: ['GER', null],
    item: { token: 'ev.2', name: 'Envoy', icon: null, description: null, options: [{ ordinal: 0, name: 'Accept' }] },
  });
  const strikeRow = row('nuclearStrike', 90, ['USA', 'JAP'], {
    category: 'warsPeace',
    participants: ['USA', 'JAP'],
    strike: { province: 1, state: null, buildingLevels: null, warSupport: -15, units: 2, manpowerLost: 1234.6, equipmentLost: null, notRecorded: ['equipment: writers not forwarded'] },
  });
  const firstRepeated = row('warJoined', 52, ['GER', 'SOV'], {
    category: 'warsPeace',
    participants: ['GER', 'SOV'],
  });
  const head = {
    branchId: '1',
    offset: '0',
    total: '4',
    live: '0',
    counts: [],
    rows: [
      { line: 'event', hour: chosenRow.hour, observed: chosenRow.hour, key: 'k0', ordinal: '0', live: false, row: chosenRow, choice: null },
      { line: 'event', hour: unknownRecipient.hour, observed: unknownRecipient.hour, key: 'k1', ordinal: '1', live: false, row: unknownRecipient, choice: null },
      { line: 'event', hour: strikeRow.hour, observed: strikeRow.hour, key: 'k2', ordinal: '2', live: false, row: strikeRow, choice: null },
      { line: 'fold', hour: firstRepeated.hour, observed: firstRepeated.hour, key: 'fold:2:GER:warJoined:1', ordinal: '3', count: '5', kind: 'warJoined', country: 'GER', firstRow: firstRepeated },
    ],
    missing: [],
    nextChange: null,
    unavailable: [],
    warnings: [],
  };
  const source = buildRowSourceFromHead('test:feed', head, undefined, 120, undefined, eventFeedProjection.validateTail)!;
  const readView = createBoundedReadView({
    hour: 120,
    rows: new Map([['main', source]]),
    identity: {
      resolve: (kind, key) => ({
        kind,
        key,
        holder: { status: 'ready', value: key },
        name: { status: 'ready', value: key === 'GER' ? 'Synthland' : key },
        validInterval: [0, 1000] as const,
        nextChangeAt: Infinity,
      }),
    },
  });
  const model = eventFeedProjection.project(readView, {
    categories: ['warsPeace', 'diplomacy', 'politics', 'milestones'],
    artwork: true,
    expanded: [],
  });
  assert.equal(model.rowHeightMode, undefined, 'event feed uses standard 36px rows');
  const chosenOut = model.rows.find(r => r.key === 'k0')!;
  assert.equal(chosenOut.cells.detail?.inlinePrefix, 'B');
  assert.deepEqual(chosenOut.cells.detail?.value, { kind: 'known', value: 'Resign' });
  const chosenMeta = chosenOut.meta as EventDetailSubject;
  assert.equal(chosenMeta.chosenOrdinal, 1);
  assert.equal(chosenMeta.chosenHour, 100);
  const unknownOut = model.rows.find(r => r.key === 'k1')!;
  const unknownMeta = unknownOut.meta as EventDetailSubject;
  assert.equal(unknownMeta.countryTag, null, 'missing recipient never substitutes the sender');
  const strikeOut = model.rows.find(r => r.key === 'k2')!;
  assert.equal(strikeOut.cells.detail?.value.kind, 'known');
  assert.match(String((strikeOut.cells.detail?.value as { value: string }).value), /^1,234 casualties · −15% war support · 2 units$/);
  const strikeTip = (strikeOut as { tooltipExtra?: () => readonly { label: string; value: { value?: unknown } }[] }).tooltipExtra?.() ?? [];
  assert.deepEqual(strikeTip.filter(r => r.label === 'War support' || r.label === 'Casualties').map(r => [r.label, r.value.value]), [
    ['Casualties', '1,234'],
    ['War support', '−15%'],
  ]);
  assert.match(String((strikeOut.cells.detail?.value as { value: string }).value), /2 units/);
  const envoyTip = (hover: { key: string; description: string | null; gaps: readonly { field: 'title' | 'description' | 'option'; ordinal?: number; key: string; reason: string }[] | null; options: (string | null)[] | null; missing: string | null } | null) => {
    const extra = eventFeedProjection.project(readView, { categories: ['warsPeace', 'diplomacy', 'politics', 'milestones'], artwork: true, hover: hover ? [hover] : [] }).rows.find(r => r.key === 'k1')?.tooltipExtra;
    return ((typeof extra === 'function' ? extra() : extra) ?? []).filter(r => r.label === 'Description' || r.label.startsWith('Option ')).map(r => `${r.label}: ${r.value.kind === 'known' ? r.value.value : r.value.kind === 'notRecorded' ? `not recorded: ${r.value.reason}` : r.value.kind}`);
  };
  assert.deepEqual(envoyTip(null), [], 'a listed row carries no description or option text until its row is read');
  assert.deepEqual(envoyTip({ key: 'k1', description: 'An envoy arrives.', gaps: null, options: ['Accept', null], missing: null }),
    ['Description: An envoy arrives.', 'Option A: Accept', 'Option B: not recorded: Option text not recorded'], 'the hovered row brings its description and every option it offers');
  assert.deepEqual(envoyTip({ key: 'k1', description: null, gaps: null, options: null, missing: 'This event is not in the log at the hour it was read' }), ['Description: not recorded: This event is not in the log at the hour it was read']);
  const foldRow = model.rows.find(r => r.key === 'group:fold:2:GER:warJoined:1')!;
  assert.equal(foldRow.name, '5 events');
  assert.deepEqual(foldRow.pageGroup, { members: 5, open: false });
  assert.equal(foldRow.children, undefined, 'folding lives on the server, never in client-attached children');

  const settings = tableSettingsOf(undefined, model.columns, LOG_TABLE_BASE);
  const flatClosed = resolveTableRows(model, settings);
  const mirrorClosed = deriveTableMirror(flatClosed, model.columns, false);
  const mirrorFold = mirrorClosed.rows.find(r => r.key === 'group:fold:2:GER:warJoined:1')!;
  assert.equal(mirrorFold.group, true);
  assert.equal(mirrorFold.open, false);
  const spokenOf = (col: string) => mirrorFold.cells.find(c => c.column === col)?.spoken;
  assert.equal(spokenOf('item'), '5 events');
  assert.equal(spokenOf('kind'), 'Joined a war');
  assert.ok(spokenOf('country')?.includes('Synthland'), 'fold screen-reader row names its country');

  const noArtModel = eventFeedProjection.project(readView, {
    categories: ['warsPeace', 'diplomacy', 'politics', 'milestones'],
    artwork: false,
    expanded: [],
  });
  const noArtMirror = deriveTableMirror(resolveTableRows(noArtModel, settings), noArtModel.columns, false).rows.find(r => r.key === 'group:fold:2:GER:warJoined:1')!;
  assert.equal(noArtMirror.cells.find(c => c.column === 'kind')?.spoken, 'Joined a war', 'screen-reader kind stays spoken even when artwork is hidden');

  const visibleCols = selectVisibleColumns(buildColumnLayouts(model.columns, settings), false);
  const auditClosed = deriveTableAudit({ model, flatRows: flatClosed, visibleColumns: visibleCols, rankShown: false });
  assert.ok(auditClosed.some(entry => entry.key === 'group:fold:2:GER:warJoined:1:item' && String(entry.value).includes('5 events')), 'exact audit compares folded lines');
});

test('expanded event-feed folds project children with pageParent, detail meta, more toggle, and declaration sources', () => {
  const foldKey = 'fold:4:GER:script-event:10';
  const child = row('scriptEventFired', 100, ['GER', 'FRA'], {
    category: 'politics',
    participants: ['GER', 'FRA'],
    item: { token: 'ev.50', name: 'Treaty Offer', icon: null, description: null, options: [{ ordinal: 0, name: 'Sign' }] },
  });
  const head = {
    branchId: '1',
    offset: '0',
    total: '3',
    live: '0',
    counts: [],
    rows: [
      { line: 'fold', key: foldKey, hour: 100, observed: 100, ordinal: '0', count: '35', kind: 'scriptEventFired', country: 'GER', firstRow: child },
      { line: 'event', key: 'native:1:50', hour: 100, observed: 100, ordinal: '1', live: false, row: child, choice: null, parent: foldKey },
      { line: 'more', key: `more:${foldKey}`, parent: foldKey, hour: 100, ordinal: '2', total: '35', remaining: '3' },
    ],
    missing: [],
    nextChange: null,
    unavailable: [],
    warnings: [],
  };
  const source = buildRowSourceFromHead('test:feed-expanded', head, undefined, 120, undefined, eventFeedProjection.validateTail)!;
  const model = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', source]]) }),
    { categories: ['politics'], artwork: true, expanded: [`group:${foldKey}`] },
  );
  assert.deepEqual(model.rows[0]!.pageGroup, { members: 35, open: true });
  assert.equal(model.rows[1]!.key, 'native:1:50');
  assert.equal(model.rows[1]!.pageParent, `group:${foldKey}`);
  assert.equal((model.rows[1]!.meta as EventDetailSubject).key, 'native:1:50', 'expanded child still opens the detail window');
  assert.equal(model.rows[2]!.key, `all:group:${foldKey}`);
  assert.equal(model.rows[2]!.pageParent, `group:${foldKey}`);
  assert.equal(model.rows[2]!.name, 'Show all 35 events · 3 more');

  const fullHead = {
    ...head,
    total: '1',
    rows: [{ line: 'more', key: `more:${foldKey}`, parent: foldKey, hour: 100, ordinal: '0', total: '35', remaining: '0' }],
  };
  const fullSource = buildRowSourceFromHead('test:feed-full', fullHead, undefined, 120, undefined, eventFeedProjection.validateTail)!;
  const fullModel = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', fullSource]]) }),
    { categories: ['politics'], artwork: true, expanded: [`group:${foldKey}`] },
  );
  assert.equal(fullModel.rows[0]!.name, 'Show fewer events');

  const plan = eventFeedDeclaration.sources({
    track: { id: 't1', source: { id: 's1', revision: 'r1' }, branch: { id: '1', firstHour: 0, lastHour: 200 }, axisOffset: 0 },
    branch: { id: '1', firstHour: 0, lastHour: 200 },
    widget: { id: 'w1', kind: 'event-feed', time: { mode: 'follow' }, countries: { kind: 'all' } },
    countries: { all: true, tags: [] },
    options: { politics: true, expanded: `group:${foldKey}`, full: `group:${foldKey}` },
    following: true,
    gateSpan: (first: number, last: number) => ({ firstHour: first, lastHour: last }),
  } as never, {} as never);
  const ask = (plan.entries[0]!.read as { tailQuery: { ask: Record<string, unknown> } }).tailQuery.ask;
  assert.deepEqual(ask.expanded, [foldKey]);
  assert.deepEqual(ask.full, [foldKey]);
});

test('event feed projection renders merged choice letter and text, awaiting choice, paused choice, and unrecorded choice ordinal from page rows', () => {
  const firedChosen = row('scriptEventFired', 100, ['GER', 'POL'], {
    category: 'politics',
    participants: ['GER', 'POL'],
    item: { token: 'ev.10', name: 'Ultimatum', icon: null, description: 'Body', options: [{ ordinal: 0, name: 'Refuse' }, { ordinal: 1, name: 'Yield' }] },
  });
  const firedAwaiting = row('newsEventFired', 104, ['GER', 'FRA'], {
    category: 'politics',
    participants: ['GER', 'FRA'],
    item: { token: 'ev.11', name: 'Border Tension', icon: null, description: null, options: [{ ordinal: 0, name: 'Mobilise' }] },
  });
  const firedPaused = row('scriptEventFired', 96, ['ITA', 'ETH'], {
    category: 'politics',
    participants: ['ITA', 'ETH'],
    item: { token: 'ev.12', name: 'Stalled Talks', icon: null, description: null, options: [{ ordinal: 0, name: 'Press on' }] },
  });
  const firedUnknownOption = row('scriptEventFired', 92, ['SOV', 'FIN'], {
    category: 'politics',
    participants: ['SOV', 'FIN'],
    item: { token: 'ev.13', name: 'Demands', icon: null, description: null, options: [{ ordinal: 0, name: 'Reject' }] },
  });
  const choiceEdge = {
    row: row('eventOptionSelected', 102, ['POL'], { category: 'politics', participants: ['POL', null] }),
    option: 1,
    lifecycle: null,
    days: null,
    costs: null,
  };
  const missingOptionEdge = {
    row: row('eventOptionSelected', 93, ['FIN'], { category: 'politics', participants: ['FIN', null] }),
    option: null,
    lifecycle: null,
    days: null,
    costs: null,
  };
  const head = {
    branchId: '1',
    offset: '0',
    total: '4',
    live: '1',
    counts: [],
    rows: [
      { line: 'event', key: 'm-chosen', hour: 100, observed: 100, ordinal: '0', live: false, status: 'chosen', row: firedChosen, choice: choiceEdge },
      { line: 'event', key: 'm-await', hour: 104, observed: 104, ordinal: '1', live: true, status: 'awaiting', row: firedAwaiting, choice: null },
      { line: 'event', key: 'm-paused', hour: 96, observed: 96, ordinal: '2', live: false, status: 'notRecorded', row: firedPaused, choice: null },
      { line: 'event', key: 'm-null-opt', hour: 92, observed: 92, ordinal: '3', live: false, status: 'chosen', row: firedUnknownOption, choice: missingOptionEdge },
    ],
    missing: [],
    nextChange: null,
    unavailable: [],
    warnings: [],
  };
  const source = buildRowSourceFromHead('test:feed-merged', head, undefined, 120)!;
  const model = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', source]]) }),
    { categories: ['politics'], artwork: true },
  );
  const chosenRow = model.rows.find(r => r.key === 'm-chosen')!;
  assert.equal(chosenRow.cells.detail?.inlinePrefix, 'B');
  assert.deepEqual(chosenRow.cells.detail?.value, { kind: 'known', value: 'Yield' });
  const chosenMeta = chosenRow.meta as EventDetailSubject;
  assert.equal(chosenMeta.chosenOrdinal, 1);
  assert.equal(chosenMeta.chosenHour, 102);
  assert.equal(chosenMeta.awaiting, false);

  const awaitingRow = model.rows.find(r => r.key === 'm-await')!;
  assert.equal(awaitingRow.cells.detail?.statusKind, 'waiting');
  assert.deepEqual(awaitingRow.cells.detail?.value, { kind: 'known', value: 'Awaiting choice' });
  assert.equal((awaitingRow.meta as EventDetailSubject).awaiting, true);

  const pausedRow = model.rows.find(r => r.key === 'm-paused')!;
  assert.deepEqual(pausedRow.cells.detail?.value, { kind: 'notRecorded', reason: 'Choice not recorded' });
  assert.equal(pausedRow.cells.detail?.missingFigure, 'Choice');
  assert.equal((pausedRow.meta as EventDetailSubject).awaiting, false);
  assert.equal((pausedRow.meta as EventDetailSubject).noChoiceReason, 'Choice not recorded');

  const reasonedHead = {
    ...head,
    rows: [
      { ...head.rows[2]!, notRecordedReason: 'event feed read in part' },
    ],
  };
  const reasonedSource = buildRowSourceFromHead('test:feed-reason', reasonedHead, undefined, 120)!;
  const reasonedModel = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', reasonedSource]]) }),
    { categories: ['politics'], artwork: true },
  );
  const reasonedRow = reasonedModel.rows.find(r => r.key === 'm-paused')!;
  assert.deepEqual(reasonedRow.cells.detail?.value, { kind: 'notRecorded', reason: 'Choice not recorded (event feed read in part)' });
  assert.equal((reasonedRow.meta as EventDetailSubject).noChoiceReason, 'Choice not recorded (event feed read in part)');

  const nullOptRow = model.rows.find(r => r.key === 'm-null-opt')!;
  assert.equal(nullOptRow.cells.detail?.value.kind, 'notRecorded');
  assert.equal(nullOptRow.cells.detail?.missingFigure, 'Choice');
  assert.equal((nullOptRow.meta as EventDetailSubject).awaiting, false);
});

test('a firing whose options come only from its choice lists them from the choice the page shows, and none before that choice', () => {
  const fired = row('newsEventFired', 100, ['GER', 'FRA'], { category: 'politics', participants: ['GER', 'FRA'], item: { token: 'ev.20', name: 'Envoy', icon: null, description: null } });
  const choice = {
    row: row('eventOptionSelected', 110, ['FRA'], { category: 'politics', participants: ['FRA', null], item: { token: 'ev.20', name: 'Envoy', icon: null, description: null, options: [{ ordinal: 0, name: 'Receive' }, { ordinal: 1, name: 'Refuse' }] } }),
    option: 1, lifecycle: null, days: null, costs: null,
  };
  const options = (hour: number, picked: typeof choice | null) => {
    const head = {
      branchId: '1', offset: '0', total: '1', live: '0', counts: [],
      rows: [{ line: 'event', key: 'm-envoy', hour: 100, observed: 100, ordinal: '0', live: false, status: picked ? 'chosen' : 'awaiting', row: fired, choice: picked }],
      missing: [], nextChange: null, unavailable: [], warnings: [],
    };
    const source = buildRowSourceFromHead(`test:feed-envoy-${hour}`, head, undefined, hour)!;
    const model = eventFeedProjection.project(createBoundedReadView({ hour, rows: new Map([['main', source]]) }), { categories: ['politics'], artwork: true, hover: [] });
    const extra = model.rows.find(r => r.key === 'm-envoy')?.tooltipExtra;
    return ((typeof extra === 'function' ? extra() : extra) ?? []).filter(r => r.label.startsWith('Option ')).map(r => `${r.label}: ${r.value.kind === 'known' ? r.value.value : r.value.kind}`);
  };
  assert.deepEqual(options(105, null), [], 'before its choice the firing offers no option text');
  assert.deepEqual(options(120, choice), ['Option A: Receive', 'Option B: Refuse'], 'after it, the choice the page shows supplies them');
});

test('an orphan choice opened from the feed has no firing time and labels its observation as chosen by', () => {
  const selected = row('eventOptionSelected', 119, ['ITA'], { category: 'politics', readHour: 137, participants: ['ITA', null] });
  const source = buildRowSourceFromHead('test:feed-orphan', { branchId: '1', offset: '0', total: '1', live: '0', counts: [],
    rows: [{ line: 'event', key: selected.id, hour: 119, observed: 137, ordinal: '0', live: false, row: selected, choice: null }],
    missing: [], unavailable: [], warnings: [] }, undefined, 137)!;
  const model = eventFeedProjection.project(createBoundedReadView({ hour: 137, rows: new Map([['main', source]]) }), { categories: ['politics'], artwork: false });
  const subject = model.rows[0]!.meta as EventDetailSubject;
  assert.equal(subject.firedHour, null);
  assert.equal(subject.chosenHour, 137);
  assert.equal(subject.noChoiceReason, 'Chosen option not recorded');
});

test('witnessed participant roles keep unidentified countries without adding an absent decision target', () => {
  const ceased = validateFeedRow(row('ceasedToExist', 100, [], { exact: false, participants: [null, null] }));
  const guarantee = validateFeedRow(row('guaranteeGranted', 101, ['NOR'], { category: 'diplomacy', exact: false, participants: ['NOR', null] }));
  assert.throws(() => validateFeedRow(row('peace', 103, ['NOR', 'SWE'])), /malformed/, 'a family row that names its countries lists no roles');
  assert.throws(() => validateFeedRow(row('ceasedToExist', 102, ['NOR'], { exact: false, participants: [null, 'NOR'] })), /malformed/,
    'a one-country row has no second role');
  const roleCases: { event: EventFeedRow; flags: string[] }[] = [
    { event: row('decisionRemoved', 100, [], { category: 'politics' }), flags: ['?'] },
    { event: row('decisionSelected', 100, [], { category: 'politics', cause: { notes: [], targetRecorded: false } }), flags: ['?'] },
    { event: row('decisionRemoved', 100, [], { category: 'politics', cause: { notes: [], targetRecorded: true } }), flags: ['?', '?'] },
    { event: row('decisionRemoved', 100, ['AAA'], { category: 'politics', cause: { state: 77, notes: [], targetRecorded: true } }), flags: ['AAA'] },
    { event: row('warDeclared', 100, []), flags: ['?', '?'] },
    { event: row('stateTransferred', 100, []), flags: ['?', '?'] },
  ];
  const all = [ceased, guarantee, ...roleCases.map(({ event }) => event)];
  const head = {
    branchId: '1', log: 'feed-events', columns: [], baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: String(all.length),
    rows: all.map((r, i) => ({ line: 'event', hour: r.hour, observed: r.hour, key: `k${i}`, ordinal: String(i), live: false, row: r, choice: null })),
    facetTotals: [], missing: [], unavailable: [], warnings: [],
  };
  const source = buildRowSourceFromHead('test:feed', head, undefined, 120, undefined, eventFeedProjection.validateTail)!;
  const readView = createBoundedReadView({
    hour: 120,
    identity: datedIdentity({ NOR: [{ from: 0, name: 'Norway' }] }),
    rows: new Map([['main', source]]),
  });
  const model = eventFeedProjection.project(readView, { categories: ['warsPeace', 'diplomacy', 'politics', 'milestones'], artwork: false });
  const country = (key: string) => model.rows.find(r => r.key === key)!.cells.country!;
  assert.deepEqual(country('k0').value, { kind: 'notRecorded', reason: 'Country not identified' }, 'one country, and it is missing, never a known string');
  assert.deepEqual(country('k0').flags?.map(flag => flag.tag), ['?']);
  assert.deepEqual(country('k1').value, { kind: 'partial', value: 'Norway', reason: 'Country not identified' }, 'the guaranteed country is missing beside the guarantor');
  assert.deepEqual(country('k1').flags?.map(flag => flag.tag), ['NOR', '?']);
  for (const [i, { flags }] of roleCases.entries()) {
    const cell = country(`k${i + 2}`);
    assert.deepEqual(cell.flags?.map(flag => flag.tag), flags, `case ${i}`);
    if (flags.every(tag => tag === '?')) assert.deepEqual(cell.value, { kind: 'notRecorded', reason: 'Country not identified' });
  }
  const tip = (key: string) => {
    const extra = model.rows.find(r => r.key === key)!.tooltipExtra;
    return ((typeof extra === 'function' ? extra() : extra) ?? []).map(r => r.label);
  };
  assert.ok(tip('k1').includes('Observed at') && !tip('k1').includes('Time'), 'an inexact row is dated by its observation, and says so');
});

test('a finished project with an unidentified owner keeps its row and carries its own missing reason in the country cell', () => {
  const firstReason = 'Owner not recorded: Countries was not complete at these hours';
  const secondReason = 'Owner not recorded: raw owner 9001 had no country link at this hour.';
  const firstUnknown = validateFeedRow(row('projectFinished', 100, [], {
    category: 'milestones',
    readHour: 105,
    participants: [null, null],
    ownerMissing: firstReason,
    item: { token: null, name: 'Modded Project', icon: null, description: null },
  }));
  const secondUnknown = validateFeedRow(row('projectFinished', 101, [], {
    category: 'milestones',
    readHour: 105,
    participants: [null, null],
    ownerMissing: secondReason,
    item: { token: null, name: 'Unlinked Project', icon: null, description: null },
  }));
  const known = validateFeedRow(row('projectFinished', 102, ['X01'], {
    category: 'milestones',
    readHour: 106,
    participants: ['X01', null],
    item: { token: null, name: 'Second Project', icon: null, description: null },
  }));
  assert.throws(() => validateFeedRow(row('projectFinished', 103, ['X01'], { category: 'milestones', participants: [null, 'X01'] })), /malformed/,
    'a project completion has no second role');
  assert.throws(() => validateFeedRow(row('projectFinished', 103, [], { category: 'milestones', participants: [null, null] })), /malformed/,
    'an unidentified project completion must carry its missing reason');
  assert.throws(() => validateFeedRow(row('projectFinished', 103, ['X01'], { category: 'milestones', participants: ['X01', null], ownerMissing: firstReason })), /malformed/,
    'a resolved project completion carries no missing-owner reason');
  assert.throws(() => validateFeedRow(row('ceasedToExist', 103, [], { exact: false, participants: [null, null], ownerMissing: firstReason })), /malformed/,
    'other kinds do not carry ownerMissing');
  const all = [firstUnknown, secondUnknown, known];
  const head = {
    branchId: '1', log: 'feed-events', columns: [], baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: String(all.length),
    rows: all.map((r, i) => ({ line: 'event', hour: r.hour, observed: r.hour, key: `p${i}`, ordinal: String(i), live: false, row: r, choice: null })),
    facetTotals: [], missing: [], unavailable: [], warnings: [],
  };
  const source = buildRowSourceFromHead('test:project-finished', head, undefined, 120)!;
  const model = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', source]]) }),
    { categories: ['milestones'], artwork: false },
  );
  const firstCountry = model.rows.find(r => r.key === 'p0')!.cells.country!;
  assert.deepEqual(firstCountry.value, { kind: 'notRecorded', reason: firstReason });
  assert.deepEqual(firstCountry.flags?.map(flag => flag.tag), ['?']);
  const secondCountry = model.rows.find(r => r.key === 'p1')!.cells.country!;
  assert.deepEqual(secondCountry.value, { kind: 'notRecorded', reason: secondReason });
  assert.deepEqual(secondCountry.flags?.map(flag => flag.tag), ['?']);
  const knownCountry = model.rows.find(r => r.key === 'p2')!.cells.country!;
  assert.deepEqual(knownCountry.flags?.map(flag => flag.tag), ['X01']);
});

test('event feed projection shows each scripted gap in place with its own reason and lists all gaps in the tooltip', () => {
  const gapped = row('scriptEventFired', 100, ['GER'], {
    category: 'politics',
    participants: ['GER', null],
    item: {
      token: 'synthetic.gap',
      name: 'Summit of \uFFFC and \uFFFC in 1938',
      icon: null,
      description: null,
      options: [{ ordinal: 0, name: 'Support \uFFFC' }],
      gaps: [
        { field: 'title', key: 'ROOT.GetLeader', reason: 'reads more than an event scope\'s country name or occurrence date' },
        { field: 'title', key: 'FROM.GetName', reason: 'event scope 1: it carries a character' },
        { field: 'option', ordinal: 0, key: 'FROM.GetName', reason: 'event scope 1: it carries a character' },
      ],
    },
  });
  const choice = {
    row: row('eventOptionSelected', 102, ['GER'], { category: 'politics', participants: ['GER', null] }),
    option: 0,
    lifecycle: null,
    days: null,
    costs: null,
  };
  const head = {
    branchId: '1',
    log: 'feed-events',
    columns: [],
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '1',
    rows: [{ line: 'event', hour: gapped.hour, observed: gapped.hour, key: 'k-gap', ordinal: '0', live: false, row: gapped, choice }],
    facetTotals: [],
    missing: [],
    unavailable: [],
    warnings: [],
  };
  const source = buildRowSourceFromHead('test:feed-gaps', head, undefined, 120)!;
  const model = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', source]]) }),
    { categories: ['politics'], artwork: false },
  );
  const out = model.rows[0]!;
  assert.equal(out.cells.item?.value.kind, 'partial');
  const shown = out.cells.item?.value.kind === 'partial' ? String(out.cells.item.value.value) : '';
  assert.equal(
    shown,
    'Summit of [not recorded: ROOT.GetLeader: reads more than an event scope\'s country name or occurrence date] and [not recorded: FROM.GetName: event scope 1: it carries a character] in 1938',
  );
  assert.equal(out.cells.detail?.value.kind, 'partial');
  assert.equal(
    out.cells.detail?.value.kind === 'partial' ? out.cells.detail.value.value : '',
    'Support [not recorded: FROM.GetName: event scope 1: it carries a character]',
  );
  const asFlat = (entry: TableRow) => ({
    kind: 'data' as const,
    key: entry.key,
    name: entry.name,
    row: entry,
    rank: 1,
    level: 0,
    parent: null,
    isTreeGroup: false,
    isOpen: false,
    childCount: 0,
  });
  const rawTip = deriveRowTooltip(asFlat(out), model.columns);
  assert.equal(rawTip.scrollable, true);
  assert.deepEqual(
    rawTip.rows
      .filter(r => r.label.startsWith('Text not recorded'))
      .map(r => [r.label, r.valueState?.kind, r.valueState?.text]),
    [
      ['Text not recorded (title)', 'known', 'ROOT.GetLeader: reads more than an event scope\'s country name or occurrence date'],
      ['Text not recorded (title)', 'known', 'FROM.GetName: event scope 1: it carries a character'],
      ['Text not recorded (option A)', 'known', 'FROM.GetName: event scope 1: it carries a character'],
    ],
  );
  const renderedTip = tooltipText(rawTip);
  assert.match(renderedTip, /Text not recorded \(title\) ROOT\.GetLeader: reads more than an event scope's country name or occurrence date/);
  assert.match(renderedTip, /Text not recorded \(title\) FROM\.GetName: event scope 1: it carries a character/);

  const manyGaps = Array.from({ length: 12 }, (_, ordinal) => ({
    field: 'option' as const,
    ordinal,
    key: `OPT_${ordinal}.GetName`,
    reason: `option ${ordinal} scope was not recorded`,
  }));
  const overflowModel = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', source]]) }),
    {
      categories: ['politics'],
      artwork: false,
      hover: [{ key: 'k-gap', description: 'Body \uFFFC', gaps: manyGaps, options: null, missing: null }],
    },
  );
  const overflowTip = deriveRowTooltip(asFlat(overflowModel.rows[0]!), overflowModel.columns);
  const fitted = fitTooltip([overflowTip], 160);
  assert.equal(fitted.scrollable, true, 'overflowing scripted gap tooltips scroll instead of dropping rows');
  assert.equal(fitted.rows.filter(r => r.label.startsWith('Text not recorded (option')).length, 12);
  assert.equal(fitted.rows.some(r => (r.omittedRows ?? 0) > 0), false, 'no gap row is replaced with "N more"');
  assert.match(tooltipText(fitted), /OPT_11\.GetName: option 11 scope was not recorded/);

  const twelfthIndex = fitted.rows.findIndex(r => r.valueState?.text?.includes('OPT_11.GetName'));
  assert.ok(twelfthIndex >= 0);
  const offsets = tooltipRowOffsets(fitted.rows);
  const viewport = 120;
  const maxScroll = offsets[offsets.length - 1]! - viewport;
  assert.ok(maxScroll > 0, 'twelve option gaps overflow the viewport');
  assert.ok(tooltipRowWindow(offsets, 0, viewport).last <= twelfthIndex, 'twelfth gap starts below the initial fold');
  showTooltip('event-feed-test', fitted, 10, 10, 'k-gap');
  measureTooltipScroll('event-feed-test', viewport);
  assert.equal(handleActiveTooltipKey('other-widget', 'PageDown', false, maxScroll), false);
  assert.equal(handleActiveTooltipWheel('other-widget', maxScroll, maxScroll), false);
  assert.equal(handleActiveTooltipKey('event-feed-test', 'PageDown', false, maxScroll), true);
  assert.equal(handleActiveTooltipWheel('event-feed-test', maxScroll, maxScroll), true);
  const afterScroll = tooltipRowWindow(offsets, tooltipStore.get()!.scrollTop!, viewport);
  assert.ok(twelfthIndex >= afterScroll.first && twelfthIndex < afterScroll.last, 'scrolling brings the twelfth gap into view');
  hideTooltip('event-feed-test');
});

test('S3: a malformed event feed row becomes a refused slot with its reason instead of disappearing or reporting no matches', () => {
  const good = row('warDeclared', 100, ['GER', 'POL']);
  const bad = { ...good, participants: ['GER', 'ITA'] };
  const mixedHead = {
    branchId: '1',
    total: '2',
    offset: '0',
    live: '0',
    counts: [],
    rows: [
      { line: 'event', key: 'ok', hour: 100, observed: 100, ordinal: '0', live: false, row: good, choice: null },
      { line: 'event', key: 'bad', hour: 100, observed: 100, ordinal: '1', live: false, row: bad, choice: null },
    ],
    missing: [],
    nextChange: null,
    unavailable: [],
    warnings: [],
  };
  const mixedSource = buildRowSourceFromHead('test:feed-mixed-bad', mixedHead, undefined, 120, undefined, eventFeedProjection.validateTail)!;
  const mixedModel = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', mixedSource]]) }),
    { categories: ['warsPeace'], artwork: false },
  );
  assert.equal(mixedModel.rows.length, 1);
  assert.equal(mixedModel.rows[0]?.key, 'ok');
  assert.equal(mixedModel.slots?.length, 2);
  assert.equal(mixedModel.slots?.[0]?.kind, 'data');
  assert.deepEqual(mixedModel.slots?.[1], { kind: 'refused', key: 'refused:1', reason: 'The event feed row is malformed.' });
  assert.equal(mixedModel.totalRows, 2);
  assert.equal(mixedModel.empty, undefined);

  const onlyBadHead = {
    ...mixedHead,
    total: '1',
    rows: [{ line: 'event', key: 'bad', hour: 100, observed: 100, ordinal: '0', live: false, row: bad, choice: null }],
  };
  const onlyBadSource = buildRowSourceFromHead('test:feed-only-bad', onlyBadHead, undefined, 120, undefined, eventFeedProjection.validateTail)!;
  const onlyBadModel = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', onlyBadSource]]) }),
    { categories: ['warsPeace'], artwork: false },
  );
  assert.equal(onlyBadModel.rows.length, 0);
  assert.deepEqual(onlyBadModel.slots, [{ kind: 'refused', key: 'refused:0', reason: 'The event feed row is malformed.' }]);
  assert.equal(onlyBadModel.totalRows, 1);
  assert.equal(onlyBadModel.empty, undefined, 'a tail with one malformed feed row never claims no events matched');

  const badChoiceHead = {
    ...mixedHead,
    total: '1',
    rows: [{ line: 'event', key: 'bad-choice', hour: 100, observed: 100, ordinal: '0', live: false, row: good, choice: { row: bad, option: 0 } }],
  };
  const badChoiceSource = buildRowSourceFromHead('test:feed-bad-choice', badChoiceHead, undefined, 120, undefined, eventFeedProjection.validateTail)!;
  const badChoiceModel = eventFeedProjection.project(
    createBoundedReadView({ hour: 120, rows: new Map([['main', badChoiceSource]]) }),
    { categories: ['warsPeace'], artwork: false },
  );
  assert.equal(badChoiceModel.rows.length, 0);
  assert.deepEqual(badChoiceModel.slots, [{ kind: 'refused', key: 'refused:0', reason: 'The event feed row is malformed.' }]);
  assert.equal(badChoiceModel.empty, undefined);

  const nullAndFutureNameHead = {
    ...mixedHead,
    total: '3',
    rows: [
      { line: 'event', key: 'null-row', hour: 100, observed: 100, ordinal: '0', live: false, row: null, choice: null },
      { line: 'event', key: 'future-name', hour: 100, observed: 100, ordinal: '1', live: false, row: { ...good, readHour: 100, nameHour: 101 }, choice: null },
      { line: 'fold', key: 'fold:4:GER:warDeclared:1', hour: 100, observed: 100, ordinal: '2', count: '5', kind: 'warDeclared', country: 'GER', firstRow: null },
    ],
  };
  const nullAndFutureSource = buildRowSourceFromHead('test:feed-null-and-name-hour', nullAndFutureNameHead, undefined, 100, undefined, eventFeedProjection.validateTail)!;
  const nullAndFutureModel = eventFeedProjection.project(
    createBoundedReadView({ hour: 100, rows: new Map([['main', nullAndFutureSource]]) }),
    { categories: ['warsPeace'], artwork: false },
  );
  assert.deepEqual(nullAndFutureModel.slots, [
    { kind: 'refused', key: 'refused:0', reason: 'Event was not recorded.' },
    { kind: 'refused', key: 'refused:1', reason: 'The event feed row is malformed.' },
    { kind: 'refused', key: 'refused:2', reason: 'Event was not recorded.' },
  ]);
});
