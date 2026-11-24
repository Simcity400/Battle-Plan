import assert from 'node:assert/strict';
import test from 'node:test';
import { familyRows, feedFacets, rowMatches, spanNotes, validateFeedRow, validateSpan } from './eventFeedData.ts';
import type { EventFeedRow, FeedKindCode } from '../../generated/dashboard';

let serial = 0;
const row = (kind: EventFeedRow['kind'], hour: number, tags: string[], more: Partial<EventFeedRow> = {}): EventFeedRow => ({
  id: `native:1:${serial++}`, hour, readHour: hour, nameHour: hour, exact: true, category: 'warsPeace', kind, tags,
  participants: [tags[0] ?? null, tags[1] ?? null], item: null, amount: null, ...more });
const family = (kind: EventFeedRow['kind'], hour: number, tags: string[], category: EventFeedRow['category'] = 'warsPeace'): EventFeedRow => ({
  id: `${kind}:${hour}:${tags.join(',')}`, hour, readHour: hour, nameHour: hour, exact: true, category, kind, tags, item: null, amount: null });

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

test('the kind facet takes the codes of the chosen categories, never a code no category holds', () => {
  const kinds: FeedKindCode[] = [{ kind: 'warDeclared', code: 0, category: 'warsPeace' }, { kind: 'scriptEventFired', code: 8, category: 'politics' },
    { kind: 'guaranteeGranted', code: 129, category: 'diplomacy' }];
  assert.deepEqual(feedFacets(kinds, new Set(['warsPeace', 'diplomacy'])), [{ name: 'kind', values: [0, 129], countries: [] }]);
  assert.equal(feedFacets(kinds, new Set()), null);
  assert.equal(feedFacets(kinds, new Set(['milestones'])), null, 'no kind of the log is a milestone: the log is not asked with an empty filter');
  assert.deepEqual(feedFacets(kinds, new Set(['milestones', 'politics'])), [{ name: 'kind', values: [8], countries: [] }]);
});

test('a row matches a chosen country through any country it names at its own hour, a vanished one at its last', () => {
  const chosen = (tag: string, hour: number) => tag === 'POL' && hour <= 150;
  assert.equal(rowMatches(row('warDeclared', 100, ['GER', 'POL']), chosen), true, 'the target');
  assert.equal(rowMatches(row('warJoined', 100, ['ITA', 'GER'], { cause: { how: null, within: null, conference: null, state: null, ally: 'POL', reason: null,
    notes: [] } }), chosen), true, 'a join\'s ally');
  assert.equal(rowMatches(family('ceasedToExist', 200, ['POL']), chosen), false);
  assert.equal(rowMatches({ ...family('ceasedToExist', 200, ['POL']), nameHour: 140 }, chosen), true, 'named at its last read with states');
});

test('span validation filters by category, range, and country and formats unavailable notes', () => {
  const span = validateSpan({ branchId: '1', firstHour: 0, lastHour: 1000, kinds: [],
    unavailable: [{ kind: 'content', label: 'Event text', reason: 'The event database is unavailable', count: 5 }],
    warnings: ['Read at 10 lost'],
    rows: [family('warStarted', 50, ['GER', 'POL']), family('factionJoined', 150, ['ITA'], 'diplomacy'), family('capitulated', 250, ['POL']),
      family('warStarted', 350, ['SOV', 'FIN'])] });
  assert.deepEqual(familyRows(span, new Set(['warsPeace']), 200, tag => tag === 'POL').map(held => held.hour), [250], 'category, range and country');
  assert.deepEqual(spanNotes(span), ['Read at 10 lost', 'Event text: Not recorded — The event database is unavailable (5 counted)']);
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

