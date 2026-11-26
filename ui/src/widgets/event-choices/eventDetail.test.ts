import assert from 'node:assert/strict';
import test from 'node:test';
import { detailRequest, hoverSubject, openedItem } from './eventDetail.ts';

const item = { token: 'news.323', name: 'King Carol II Abdicates', icon: null, description: 'Pressured by powerful elements…', options: [{ ordinal: 0, name: 'A new king' }] };
const row = { id: 'native:1:7', hour: 100, readHour: 100, nameHour: 100, exact: true, category: 'politics', kind: 'newsEventFired', tags: ['ROM'], participants: ['ROM', null], item, amount: null };

test('an opened event row brings its full text, and a row not in the log at its hour says so', () => {
  const card = { rows: [{ key: 'native:1:7', hour: 100, observed: 100, ordinal: '0', live: true, status: 'awaiting', occurrence: { id: 'native:1:7', opened: true, edges: [{ row, option: null, lifecycle: null, days: null, costs: null }] } }] };
  assert.deepEqual(openedItem('event-choices-tail', card), { pending: false, missing: null, item });
  assert.deepEqual(openedItem('decisions-tail', { ...card, unavailable: [{ kind: 'scriptEffectDescription', label: 'Authored effects', reason: 'GER_war_bonds complete_effect: inputs not recorded', count: 1 }] }).effectsMissing, ['GER_war_bonds complete_effect: inputs not recorded'], 'an opened row names the effects it could not describe');
  assert.deepEqual(openedItem('event-feed-tail', { rows: [{ key: 'native:1:7', hour: 100, observed: 100, ordinal: '0', live: false, row, choice: null }] }).item, item);
  const chosen = { ...item, token: 'news.323.b', description: 'The new king is crowned.' };
  assert.equal(openedItem('event-feed-tail', { rows: [{ key: 'native:1:8', hour: 100, observed: 100, ordinal: '0', live: false, row: { ...row, item: null }, choice: { row: { ...row, item: chosen }, option: 0, lifecycle: null, days: null, costs: null } }] }).item, chosen, 'a feed row without its own item opens with the item of its chosen option, as the list shows it');
  const own = openedItem('event-feed-tail', { rows: [{ key: 'native:1:8', hour: 100, observed: 100, ordinal: '0', live: false, row: { ...row, item: { ...item, options: undefined } }, choice: { row: { ...row, item: chosen }, option: 0, lifecycle: null, days: null, costs: null } }] });
  assert.deepEqual([own.item?.options?.length, own.own?.options], [1, undefined], 'an opened feed row keeps its own options apart from those its choice supplies, so a later choice never fills a tooltip read for an earlier page');
  const gone = openedItem('event-choices-tail', { rows: [] });
  assert.equal(gone.item, null);
  assert.match(gone.missing ?? '', /not in the log/);
});

test('a hovered row is read once, at the hour the pointer reached it, and playback asks nothing more', () => {
  const track = { id: 't', source: { id: 's', revision: 'r1' }, branch: { id: '1', firstHour: 0, lastHour: 500 }, axisOffset: 24 } as never;
  let hour = 100;
  const clock = { getHour: () => hour };
  const isEvent = (key: string) => !key.startsWith('fold:');
  const asked = new Set<string>();
  const ask = (subject: ReturnType<typeof hoverSubject>) => { const request = detailRequest(track, 'decisions-tail', subject); if (request) asked.add(JSON.stringify(request)); };
  const ger = hoverSubject(track, clock, true, isEvent, 'native:1:7');
  assert.equal(ger?.hour, 124, 'the read hour is the displayed recording hour when the pointer arrived');
  for (; hour < 110; hour++) ask(ger);
  assert.equal(asked.size, 1, 'ten played hours under a resting pointer ask one read');
  ask(hoverSubject(track, clock, true, isEvent, 'native:1:8'));
  assert.equal(asked.size, 2, 'the next row is one more read');
  assert.equal(hoverSubject(track, clock, true, isEvent, 'fold:native:1:7'), null, 'a folded group names no event and is not read');
  assert.equal(hoverSubject(track, clock, true, isEvent, null), null);
  assert.equal(hoverSubject(track, clock, false, isEvent, 'native:1:7')?.hour, 500, 'a pinned widget reads at the recording end, where every listed row is in the log');
});
