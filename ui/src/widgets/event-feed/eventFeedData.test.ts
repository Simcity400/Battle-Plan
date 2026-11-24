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
  const event = row('warDeclared', 100, ['GER', 'POL'], { readHour: 101, cause: { how: 'a diplomatic action', within: null, conference: null, state: null,
    ally: null, reason: null, notes: [] } });
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
    cause: { how: null, within: null, conference: null, state: null, ally: null, reason: null, notes: ['Selected native option ordinal: 1.'] },
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
  const foldRow = model.rows.find(r => r.key.startsWith('fold:'))!;
  assert.equal(foldRow.name, '5 events');
  assert.equal(foldRow.children?.length, 5);
  assert.equal(foldRow.initiallyOpen, false);
});

