import assert from 'node:assert/strict';
import test from 'node:test';
import { factsKey, HourStore, nameKey, transientError } from './countryFacts.ts';
import { lookHour, LOOK_STEP } from './countryLooks.ts';
import { nextLooks } from './looks.ts';
import type { Look, LookRead } from './looks';
import { TRANSIENT_ERRORS } from '../generated/dashboard.ts';

const germany: Look = { name: 'Germany', color: [1, 2, 3], flag: 'data:ger' };
const style = { name: 'Germany', color: [1, 2, 3] as [number, number, number] };

test('each part of a look still being read keeps the one shown; a completed read replaces it, with nothing known when it gave nothing', () => {
  const shown = new Map<string, Look>([['GER', germany]]);
  const read = (value: LookRead) => nextLooks(shown, ['GER'], () => value).get('GER');
  // Loading the next month's look: the one shown stays, so nothing flashes.
  assert.equal(read({ style: undefined, flag: undefined }), germany);
  // Read, and another country now holds the tag: its look replaces the old one.
  assert.deepEqual(read({ style: { name: 'German Reich', color: [9, 9, 9] }, flag: 'data:reich' }), { name: 'German Reich', color: [9, 9, 9], flag: 'data:reich' });
  // Read with nothing: not known, never the older look.
  assert.deepEqual(read({ style: null, flag: null }), { name: null, color: null, flag: null });
  // A country not asked about keeps its look.
  assert.equal(nextLooks(shown, ['ITA'], () => ({ style: null, flag: null })).get('GER'), germany);
});

test('a style applies on its own: flags that cannot be had (no game folders) leave the colours shown', () => {
  // Styles read; flags completed with nothing (the product needs content): colours and
  // names show, the flag is the placeholder.
  const looks = nextLooks(new Map(), ['GER'], () => ({ style, flag: null }));
  assert.deepEqual(looks.get('GER'), { name: 'Germany', color: [1, 2, 3], flag: null });
  // Styles read while flags are still being read: the style is not held back.
  assert.deepEqual(nextLooks(new Map(), ['GER'], () => ({ style, flag: undefined })).get('GER'), { name: 'Germany', color: [1, 2, 3], flag: null });
  // A flag read later joins the style already shown.
  const later = nextLooks(looks, ['GER'], () => ({ style: undefined, flag: 'data:ger' }));
  assert.deepEqual(later.get('GER'), germany);
});

test('only refusals for want of room, cancellations and an unreachable service are tried again', () => {
  for (const refusal of TRANSIENT_ERRORS) {
    assert.equal(transientError(new Error(refusal)), true, refusal);
    // The desktop host's own refusal starts with the service's.
    assert.equal(transientError(`${refusal}. Retry this view.`), true);
  }
  assert.equal(transientError(new DOMException('Request cancelled', 'AbortError')), true);
  assert.equal(transientError(new TypeError('Failed to fetch')), true);
  // The product's own answers: asking again gives them again.
  assert.equal(transientError(new Error('Choose the game and user folders in Settings first')), false);
  assert.equal(transientError(new Error('Country styles unavailable at this hour: gap')), false);
});

test('looks are read at a recorded hour of the moment’s own stretch, never in a gap', () => {
  const branch = { firstHour: 100, lastHour: 3000, gaps: [{ firstHour: 820, lastHour: 820 }, { firstHour: 1600, lastHour: 1700 }] };
  // Whole steps from the branch start while they are recorded.
  assert.deepEqual(lookHour(branch, 500), { hour: 100, stretch: 100 });
  // The step (820) is the gap: the stretch after it starts at 821.
  assert.deepEqual(lookHour(branch, 821), { hour: 821, stretch: 821 });
  assert.deepEqual(lookHour(branch, 1500), { hour: 821, stretch: 821 });
  assert.deepEqual(lookHour(branch, 1550), { hour: 100 + 2 * LOOK_STEP, stretch: 821 });
  // In a gap, nothing is read; after it, the stretch's own first hour.
  assert.equal(lookHour(branch, 1650), null);
  assert.deepEqual(lookHour(branch, 1701), { hour: 1701, stretch: 1701 });
  assert.equal(lookHour(branch, 50), null);
  assert.equal(lookHour(branch, 3001), null);
});

test('facts are asked for by the set of countries, so a re-ranking never cancels a read', () => {
  const ranked = [{ hour: 10, tag: 'GER' }, { hour: 10, tag: 'ITA' }, { hour: 10, tag: 'SOV' }];
  const reordered = [ranked[2], ranked[0], ranked[1], ranked[0]];
  assert.equal(factsKey('track', reordered), factsKey('track', ranked));
  assert.notEqual(factsKey('track', ranked.slice(1)), factsKey('track', ranked));
  assert.notEqual(factsKey('other', ranked), factsKey('track', ranked));
  // Another hour is another request, and a tag holding any character stays apart.
  assert.notEqual(factsKey('track', [{ hour: 11, tag: 'GER' }]), factsKey('track', [{ hour: 10, tag: 'GER' }]));
  assert.notEqual(factsKey('track', [{ hour: 10, tag: 'A","B' }]), factsKey('track', [{ hour: 10, tag: 'A' }, { hour: 10, tag: 'B' }]));
});

test('the fact store tells a read without a value from one still going', () => {
  const store = new HourStore<string>(4);
  const wanted = [{ hour: 10, tag: 'GER' }, { hour: 10, tag: 'ITA' }, { hour: 20, tag: 'GER' }];
  store.record(10, ['GER', 'ITA'], new Map([['GER', 'Germany']]));
  const read = store.pickRead(wanted);
  assert.equal(read.get(nameKey(10, 'GER')), 'Germany');
  assert.equal(read.get(nameKey(10, 'ITA')), null);
  assert.equal(read.has(nameKey(20, 'GER')), false);
  // The plain pick leaves both out.
  assert.deepEqual([...store.pick(wanted).keys()], [nameKey(10, 'GER')]);
});
