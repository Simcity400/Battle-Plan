import assert from 'node:assert/strict';
import test from 'node:test';
import { openedItem } from './eventDetail.ts';

const item = { token: 'news.323', name: 'King Carol II Abdicates', icon: null, description: 'Pressured by powerful elements…', options: [{ ordinal: 0, name: 'A new king' }] };
const row = { id: 'native:1:7', hour: 100, readHour: 100, nameHour: 100, exact: true, category: 'politics', kind: 'newsEventFired', tags: ['ROM'], participants: ['ROM', null], item, amount: null };

test('an opened event row brings its full text, and a row not in the log at its hour says so', () => {
  const card = { rows: [{ key: 'native:1:7', hour: 100, observed: 100, ordinal: '0', live: true, status: 'awaiting', occurrence: { id: 'native:1:7', opened: true, edges: [{ row, option: null, lifecycle: null, days: null, costs: null }] } }] };
  assert.deepEqual(openedItem('event-choices-tail', card), { pending: false, missing: null, item });
  assert.deepEqual(openedItem('event-feed-tail', { rows: [{ key: 'native:1:7', hour: 100, observed: 100, ordinal: '0', live: false, row, choice: null }] }).item, item);
  const gone = openedItem('event-choices-tail', { rows: [] });
  assert.equal(gone.item, null);
  assert.match(gone.missing ?? '', /not in the log/);
});
