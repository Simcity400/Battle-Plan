import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import { validateFeedRow } from './eventFeedData.ts';
import { eventFeedProjection } from './eventFeedProjection.ts';
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

test('event feed projection preserves selected option ordinal, keeps null recipient distinct from sender, restores nuclear strike effects, and folds five-row runs', () => {
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
  const repeated = Array.from({ length: 5 }, (_, i) => row('warJoined', 48 + i, ['GER', 'SOV'], {
    category: 'warsPeace',
    participants: ['GER', 'SOV'],
  }));
  const all = [chosenRow, unknownRecipient, strikeRow, ...repeated];
  const head = {
    branchId: '1',
    log: 'feed-events',
    columns: [],
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: String(all.length),
    rows: all.map((r, i) => ({ hour: r.hour, key: `k${i}`, ordinal: String(i), cells: [], row: r })),
    facetTotals: [],
    missing: [],
    unavailable: [],
    warnings: [],
  };
  const source = buildRowSourceFromHead('test:feed', head, undefined, 120)!;
  const readView = createBoundedReadView({ hour: 120, rows: new Map([['main', source]]) });
  const model = eventFeedProjection.project(readView, {
    categories: ['warsPeace', 'diplomacy', 'politics', 'milestones'],
    artwork: true,
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
  const envoyTip = (hover: { key: string; description: string | null; gaps: string | null; options: (string | null)[] | null; missing: string | null } | null) => {
    const extra = eventFeedProjection.project(readView, { categories: ['warsPeace', 'diplomacy', 'politics', 'milestones'], artwork: true, hover: hover ? [hover] : [] }).rows.find(r => r.key === 'k1')?.tooltipExtra;
    return ((typeof extra === 'function' ? extra() : extra) ?? []).filter(r => r.label === 'Description' || r.label.startsWith('Option ')).map(r => `${r.label}: ${r.value.kind === 'known' ? r.value.value : r.value.kind === 'notRecorded' ? `not recorded: ${r.value.reason}` : r.value.kind}`);
  };
  assert.deepEqual(envoyTip(null), [], 'a listed row carries no description or option text until its row is read');
  assert.deepEqual(envoyTip({ key: 'k1', description: 'An envoy arrives.', gaps: null, options: ['Accept', null], missing: null }),
    ['Description: An envoy arrives.', 'Option A: Accept', 'Option B: not recorded: Option text not recorded'], 'the hovered row brings its description and every option it offers');
  assert.deepEqual(envoyTip({ key: 'k1', description: null, gaps: null, options: null, missing: 'This event is not in the log at the hour it was read' }), ['Description: not recorded: This event is not in the log at the hour it was read']);
  const foldRow = model.rows.find(r => r.key.startsWith('fold:'))!;
  assert.equal(foldRow.name, '5 events');
  assert.equal(foldRow.children?.length, 5);
  assert.equal(foldRow.initiallyOpen, false);
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
      { key: 'm-chosen', hour: 100, observed: 100, ordinal: '0', live: false, status: 'chosen', row: firedChosen, choice: choiceEdge },
      { key: 'm-await', hour: 104, observed: 104, ordinal: '1', live: true, status: 'awaiting', row: firedAwaiting, choice: null },
      { key: 'm-paused', hour: 96, observed: 96, ordinal: '2', live: false, status: 'notRecorded', row: firedPaused, choice: null },
      { key: 'm-null-opt', hour: 92, observed: 92, ordinal: '3', live: false, status: 'chosen', row: firedUnknownOption, choice: missingOptionEdge },
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
  assert.equal(pausedRow.cells.detail?.value.kind, 'notRecorded');
  assert.equal(pausedRow.cells.detail?.missingFigure, 'Choice');
  assert.equal((pausedRow.meta as EventDetailSubject).awaiting, false);

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
      rows: [{ key: 'm-envoy', hour: 100, observed: 100, ordinal: '0', live: false, status: picked ? 'chosen' : 'awaiting', row: fired, choice: picked }],
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
