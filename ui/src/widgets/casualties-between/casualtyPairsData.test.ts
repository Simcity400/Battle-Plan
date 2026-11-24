import assert from 'node:assert/strict';
import test from 'node:test';
import { CasualtyPairsCursor, casualtyPairsAt, casualtyPairTags, casualtyPairWindows, pairAmount, validateCasualtyPairs as validate } from './casualtyPairsData.ts';
import { deepFreeze } from '../../testFrozen.ts';

const validateCasualtyPairs = (value: unknown) => validate(deepFreeze(value));

function fixture() {
  return { branchId: '1', available: true, windows: [{ firstHour: 100, lastHour: 120 }, { firstHour: 121, lastHour: 150 }],
    window: 0, observed: [{ firstHour: 100, lastHour: 105 }, { firstHour: 110, lastHour: 120 }], hasGap: true, warnings: [],
    pairs: [{ identity: '1:10|2:20', firstCountry: 'mod attacker', secondCountry: 'mod victim', runs: [
      { firstHour: 100, lastHour: 105, readHour: 100, hourly: true, firstTaken: '9007199254740993', secondTaken: '-12', incomplete: false },
      { firstHour: 110, lastHour: 110, readHour: 110, hourly: true, firstTaken: '0', secondTaken: '3', incomplete: true },
      { firstHour: 111, lastHour: 120, readHour: 110, hourly: false, firstTaken: '0', secondTaken: '3', incomplete: true },
    ] }] };
}
test('country-pair flows preserve direction, wide signed values and known zero', () => {
  const reply = validateCasualtyPairs(deepFreeze(fixture()));
  const at = casualtyPairsAt(reply, 105);
  assert.equal(at.flows[0].from, 'mod attacker');
  assert.equal(at.flows[0].to, 'mod victim');
  assert.equal(at.flows[0].value, -12n);
  assert.equal(at.flows[1].value, 9007199254740993n);
  assert.equal(at.flows[0].readHour, 105);
  assert.equal(pairAmount(at.flows[0]), '-12');
  assert.equal(pairAmount(casualtyPairsAt(reply, 112).flows[1]), 'Recorded subtotal · 0');
  assert.equal(casualtyPairsAt(reply, 112).flows[0].readHour, 110);
  assert.equal(at.flows.some(flow => flow.from === flow.to), false);
});
test('country-pair flows never carry across gaps and selections support either or both endpoints', () => {
  const reply = validateCasualtyPairs(fixture());
  assert.deepEqual(casualtyPairsAt(reply, 108).flows, []);
  assert.equal(casualtyPairsAt(reply, 105, tag => tag === 'mod attacker').flows.length, 2);
  assert.deepEqual(casualtyPairsAt(reply, 105, tag => tag === 'mod attacker', true).flows, []);
  assert.deepEqual(casualtyPairsAt(reply, 121).flows, []);
});
test("a chord names every country its held windows pair, a later window's too, once each and in tag order", () => {
  const first = validateCasualtyPairs(fixture());
  const next = fixture(); Object.assign(next, { window: 1, observed: [{ firstHour: 121, lastHour: 150 }] });
  next.pairs = [{ identity: '1:10|3:30', firstCountry: 'mod attacker', secondCountry: 'a later victim', runs: [
    { firstHour: 130, lastHour: 150, readHour: 130, hourly: true, firstTaken: '1', secondTaken: '2', incomplete: false }] }];
  assert.deepEqual(casualtyPairTags([first, validateCasualtyPairs(next)]), ['a later victim', 'mod attacker', 'mod victim']);
  assert.deepEqual(casualtyPairTags([first, undefined]), ['mod attacker', 'mod victim'], 'a window still loading names nothing');
  assert.deepEqual(casualtyPairTags([]), []);
});
test('country-pair windows come from the recording and prefetch only during playback', () => {
  const reply = validateCasualtyPairs(fixture());
  assert.deepEqual(casualtyPairWindows(reply, 110, false), [0]);
  assert.deepEqual(casualtyPairWindows(reply, 110, true), [0, 1]);
  assert.deepEqual(casualtyPairWindows(reply, 150, true), [1]);
  assert.deepEqual(casualtyPairWindows(reply, 99, true), []);
});

test('snapshot cursors hold immutable runs and filter identities, updating reads, gaps and new replies', () => {
  const reply = validateCasualtyPairs(deepFreeze(fixture()));
  const cursor = new CasualtyPairsCursor();
  const tags: ReadonlySet<string> = deepFreeze(new Set(['mod attacker']));
  const first = cursor.at(reply, 111, tags);
  assert.equal(cursor.at(reply, 119, tags), first);
  assert.throws(() => Object.assign(reply.pairs[0].runs[2], { secondTaken: '99' }), TypeError);
  assert.throws(() => (tags as Set<string>).add('changed'), TypeError);
  assert.deepEqual(first, casualtyPairsAt(reply, 119, tag => tags.has(tag)));
  const changed: ReadonlySet<string> = deepFreeze(new Set(['not a participant']));
  assert.deepEqual(cursor.at(reply, 119, changed), { countries: [], flows: [] });
  assert.equal(cursor.at(reply, 119, tags).flows.length, 2);
  assert.deepEqual(cursor.at(reply, 119, tags, true), { countries: [], flows: [] });
  assert.deepEqual(cursor.at(reply, 108), { countries: [], flows: [] });
  const hourly = cursor.at(reply, 100);
  const later = cursor.at(reply, 101);
  assert.equal(later.countries, hourly.countries);
  assert.equal(later.flows[0].readHour, 101);
  assert.notEqual(later.flows, hourly.flows);
  const replacement = fixture();
  replacement.pairs[0].runs[2].secondTaken = '20';
  const updated = cursor.at(validateCasualtyPairs(deepFreeze(replacement)), 115);
  assert.equal(updated.flows[0].value, 20n);
  assert.deepEqual(cursor.at(reply, 121), { countries: [], flows: [] });
});


test('filtered cursors ignore excluded hourly changes but retain selected boundaries and observation gaps', () => {
  const span = { firstHour: 100, lastHour: 120 }, run = { ...span, readHour: 100, hourly: false,
    firstTaken: '12', secondTaken: '20', incomplete: false };
  let excludedReads = 0;
  const excluded = deepFreeze(Array.from({ length: 21 }, (_, index) => ({ ...run,
    firstHour: 100 + index, lastHour: 100 + index, readHour: 100 + index, secondTaken: String(index) })));
  const selected = deepFreeze(new Set(['source', 'target']));
  const reply = validateCasualtyPairs({ branchId: '1', available: true, windows: [span], window: 0, hasGap: false, observed: [span], warnings: [],
    pairs: [{ identity: 'selected', firstCountry: 'source', secondCountry: 'target', runs: [run] },
      { identity: 'excluded', firstCountry: 'other', secondCountry: 'unselected', get runs() { excludedReads++; return excluded; } },
      { identity: 'one endpoint', firstCountry: 'source', secondCountry: 'other', runs: excluded }] });
  const cursor = new CasualtyPairsCursor();
  excludedReads = 0;
  const first = cursor.at(reply, 100, selected, true);
  for (let hour = 101; hour <= 120; hour++) assert.equal(cursor.at(reply, hour, selected, true), first);
  assert.equal(excludedReads, 0);
  assert.deepEqual(first, casualtyPairsAt(reply, 120, tag => selected.has(tag), true));
  assert.equal(cursor.at(reply, 121, selected, true).flows.length, 0);
  const either = new CasualtyPairsCursor(), oneEndpoint = deepFreeze(new Set(['target']));
  const eitherFirst = either.at(reply, 100, oneEndpoint);
  assert.equal(either.at(reply, 120, oneEndpoint), eitherFirst);
  assert.equal(either.at(reply, 120, selected).flows.length, 4);
  assert.equal(either.at(reply, 120, selected).flows[2].value, 20n);
  const gapReply = validateCasualtyPairs(fixture()), gapCursor = new CasualtyPairsCursor();
  assert.equal(gapCursor.at(gapReply, 105).flows.length, 2);
  assert.equal(gapCursor.at(gapReply, 106).flows.length, 0);
  assert.equal(gapCursor.at(gapReply, 110).flows[0].value, 3n);
});
test('directed pair IDs follow native holders through renames and refuse overlapping aliases', () => {
  const data = fixture();
  data.pairs[0].runs = [data.pairs[0].runs[0]];
  const alias = { ...data.pairs[0], firstCountry: 'renamed attacker', runs: [
    { ...data.pairs[0].runs[0], firstHour: 110, lastHour: 120, readHour: 110 },
  ] };
  data.pairs.push(alias);
  const reply = validateCasualtyPairs(deepFreeze(data));
  const before = casualtyPairsAt(reply, 105).flows[0];
  const after = casualtyPairsAt(reply, 115).flows[0];
  assert.equal(before.id, after.id);
  assert.notEqual(before.from, after.from);
  assert.notEqual(before.id, casualtyPairsAt(validateCasualtyPairs({ ...data, pairs: [{ ...alias, identity: '1:11|2:20' }] }), 115).flows[0].id);
  const overlapping = deepFreeze({ ...data, pairs: [data.pairs[0], { ...alias,
    runs: [{ ...alias.runs[0], firstHour: 105, lastHour: 105, readHour: 105 }] }] });
  assert.throws(() => validateCasualtyPairs(overlapping), /malformed/, 'two simultaneous aliases cannot supply one native pair');
});
test('malformed country-pair replies refuse, including overlapping tag holders', () => {
  const acrossGap = fixture(); acrossGap.pairs[0].runs[0].lastHour = 110;
  assert.throws(() => validateCasualtyPairs(acrossGap));
  const badRead = fixture(); badRead.pairs[0].runs[0].readHour = 101;
  assert.throws(() => validateCasualtyPairs(badRead));
  const carryAcrossGap = fixture(); carryAcrossGap.pairs[0].runs[2].readHour = 105;
  assert.throws(() => validateCasualtyPairs(carryAcrossGap));
  const inexact = fixture(); inexact.pairs[0].runs[0].secondTaken = '1.5';
  assert.throws(() => validateCasualtyPairs(inexact));
  const tooWide = fixture(); tooWide.pairs[0].runs[0].secondTaken = (1n << 127n).toString();
  assert.throws(() => validateCasualtyPairs(tooWide));
  const duplicate = fixture(); duplicate.pairs.push(duplicate.pairs[0]);
  assert.throws(() => validateCasualtyPairs(duplicate));
  const holders = fixture(); holders.pairs.push({ ...holders.pairs[0], identity: '1:11|2:20' });
  assert.throws(() => validateCasualtyPairs(holders));
  const manifest = fixture(); Object.assign(manifest, { window: null });
  assert.throws(() => validateCasualtyPairs(manifest));
});
