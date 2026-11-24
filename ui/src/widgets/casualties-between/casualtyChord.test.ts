import assert from 'node:assert/strict';
import test from 'node:test';
import { CasualtyChordCache, CasualtyChordSegmentCache, casualtyChord, chordNameHour, chordNamesAsked, chordPairs, OTHER_ID, pairTooltip } from './casualtyChord.ts';
import { deepFreeze } from '../../testFrozen.ts';
import { CasualtyPairsCursor, validateCasualtyPairs } from './casualtyPairsData.ts';
import type { PairFlow } from './casualtyPairsData';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';

const labels = deepFreeze((tag: string) => deepFreeze({ name: `Name ${tag}` }));
const color = deepFreeze(() => '#888888');


test('immutable chord identities retain geometry without rereading labels or flows', () => {
  const cache = new CasualtyChordCache();
  let labelReads = 0, flowReads = 0;
  const dated = deepFreeze((tag: string) => { labelReads++; return labels(tag); });
  const snapshot = deepFreeze({ countries: ['source', 'target'], flows: [
    { id: 'flow', from: 'source', to: 'target', get value() { flowReads++; return 12n; }, readHour: 100, incomplete: false },
    { id: 'correction', from: 'target', to: 'source', value: -4n, readHour: 100, incomplete: false },
  ] });
  const first = cache.at(snapshot, 1, 100, dated, color, '#777777');
  assert.deepEqual(first, casualtyChord(snapshot, 1, 100, dated, color, '#777777'));
  assert.throws(() => Object.assign(snapshot.flows[1], { value: -8n }), TypeError);
  const reads = [labelReads, flowReads];
  for (let hour = 101; hour <= 200; hour++) {
    const next = cache.at(snapshot, 1, hour, dated, color, '#777777');
    assert.equal(cache.at(snapshot, 1, hour, dated, color, '#777777'), next);
    assert.equal(next.data, first.data);
    assert.equal(next.nodes, first.nodes);
    assert.equal(next.flows, first.flows);
    assert.equal(next.hour, hour);
  }
  assert.deepEqual([labelReads, flowReads], reads);
  const replacement = deepFreeze({ ...snapshot, flows: snapshot.flows.map(flow => ({ ...flow, value: 20n })) });
  const updated = cache.at(replacement, 1, 201, dated, color, '#777777');
  assert.notEqual(updated.data, first.data);
  assert.deepEqual(updated, casualtyChord(replacement, 1, 201, dated, color, '#777777'));
  const renamed = cache.at(replacement, 1, 201, tag => ({ name: `Renamed ${tag}`, flagPending: true }), color, '#777777');
  assert.notEqual(renamed.data, updated.data);
  assert.equal(renamed.data.nodes[0].countryFlag?.flagPending, true);
  assert.notEqual(cache.at(replacement, 2, 201, dated, color, '#777777').data, renamed.data);
});

test('hourly exact reads reuse their immutable run geometry', () => {
  const reply = validateCasualtyPairs(deepFreeze({ branchId: '1', available: true, windows: [{ firstHour: 100, lastHour: 120 }],
    window: 0, observed: [{ firstHour: 100, lastHour: 120 }], hasGap: false, warnings: [], pairs: [
      { identity: 'native pair', firstCountry: 'source', secondCountry: 'target', runs: [
        { firstHour: 100, lastHour: 120, readHour: 100, hourly: true, firstTaken: '12', secondTaken: '4', incomplete: false },
      ] },
      { identity: 'held pair', firstCountry: 'held source', secondCountry: 'held target', runs: [
        { firstHour: 110, lastHour: 120, readHour: 110, hourly: false, firstTaken: '3', secondTaken: '5', incomplete: false },
      ] },
    ] }));
  const cursor = new CasualtyPairsCursor(), cache = new CasualtyChordCache(), segments = new CasualtyChordSegmentCache();
  const firstSnapshot = deepFreeze(cursor.at(reply, 110));
  const first = cache.at(firstSnapshot, 1, 110, labels, color, '#777777');
  const nextSnapshot = deepFreeze(cursor.at(reply, 111));
  const next = cache.at(nextSnapshot, 1, 111, labels, color, '#777777');
  assert.equal(segments.at(firstSnapshot, 'source', 100, 0), segments.at(nextSnapshot, 'source', 100, 0));
  assert.equal(next.data, first.data);
  assert.equal(next.hour, 111);
  assert.equal([...next.exact.values()][0].readHour, 111);
  assert.equal([...next.exact.values()].at(-1)!.readHour, 110);
  assert.deepEqual(next, casualtyChord(nextSnapshot, 1, 111, labels, color, '#777777'));
});

test('chord folding retains every exact constituent, signed correction and zero', () => {
  const flows: readonly PairFlow[] = deepFreeze(Array.from({ length: 40 }, (_, i) => ({ id: `pair ${i}`, from: 'acting', to: `mod enemy ${i}`,
    value: i === 0 ? -12n : i === 1 ? 0n : 9007199254740993n + BigInt(i), readHour: 100 + i, incomplete: i === 2 })));
  const chord = casualtyChord({ countries: ['acting', ...flows.map(flow => flow.to)], flows }, 1, 150, labels, color, '#777777');
  assert.equal(chord.exact.size, flows.length);
  assert.deepEqual([...chord.flows.values()].flat().sort(), flows.map(flow => flow.id).sort());
  assert.equal(chordPairs({ kind: 'node', id: OTHER_ID }, chord).length, flows.length);
  const foldedFlow = chord.data.flows[0];
  const members = chordPairs({ kind: 'flow', id: foldedFlow.id }, chord);
  assert.equal(members[0].value, -12n);
  assert.equal(members[1].value, 0n);
  assert.match(pairTooltip(members, labels, 150).rows[2].value!, /Recorded subtotal.*9,007,199,254,740,995/);
  assert.match(pairTooltip(members, labels, 150).rows[0].value!, /-12/);
});
test('corrections never become positive geometry and opaque tags cannot collide with Other', () => {
  const flow: PairFlow = { id: 'signed', from: '[]', to: 'opaque|country', value: -9007199254740993n, readHour: 100, incomplete: false };
  const chord = casualtyChord({ countries: [flow.from, flow.to], flows: [flow] }, Number.MAX_SAFE_INTEGER, 100, labels, color, '#777777');
  assert.equal(chord.data.flows[0].value, 0);
  assert.equal(chord.exact.get('signed')?.value, -9007199254740993n);
  assert.equal(chord.nodes.has(OTHER_ID), false);
  assert.equal(chord.countries.get(chord.data.nodes[0].id), flow.from);
});

test('native chord flags and pair hovers retain the shown read status; Other has no country flag', () => {
  const refused = deepFreeze({ name: 'Dated acting country', flagReason: 'Flag refused at read 100', flagPending: false });
  const pending = deepFreeze({ name: 'Dated enemy', flagPending: true });
  const dated: CountryLabel = tag => tag === 'acting' ? refused : pending;
  const flow: PairFlow = { id: 'correction', from: 'acting', to: 'enemy', value: -9007199254740993n, readHour: 100, incomplete: false };
  const snapshot = deepFreeze({ countries: [flow.from, flow.to], flows: [flow] });
  const chord = casualtyChord(snapshot, 2, 100, dated, color, '#777777');
  assert.equal(chord.data.nodes[0].countryFlag, refused);
  assert.equal(chord.data.nodes[1].countryFlag, pending);
  assert.equal(chord.data.nodes[0].flag, undefined, 'a native flag payload survives without artwork');
  const tip = pairTooltip(chordPairs({ kind: 'flow', id: chord.data.flows[0].id }, chord), chord.labels, chord.hour);
  assert.equal(tip.rows[0].countryFlag, refused);
  assert.equal(tip.rows[0].value, '-9,007,199,254,740,993');
  assert.equal(tip.note, undefined);
  assert.equal(chord.data.flows[0].value, 0, 'the correction remains selectable without a positive ribbon');
  const later = casualtyChord(snapshot, 2, 200, tag => ({ name: `Later ${tag}`, flag: 'later flag', flagPending: false }), color, '#777777');
  assert.equal(later.data.nodes[0].countryFlag?.flag, 'later flag');
  assert.equal(chord.data.nodes[0].countryFlag, refused, 'a later flag cannot backfill the pinned read');
  const folded = casualtyChord(snapshot, 1, 100, dated, color, '#777777');
  assert.equal(folded.data.nodes.find(node => node.id === OTHER_ID)?.countryFlag, undefined);
});

test("following, a chord names its windows' countries at the hour one of their identities last changed, and each frame there, exactly", () => {
  const windowTags = deepFreeze(['ENG', 'GER', 'SOV']), shownTags = deepFreeze(['ENG', 'GER']);
  for (const hour of [30_240, 30_960, 31_680]) {
    const asked = chordNamesAsked(true, 29_000, hour, windowTags, shownTags);
    assert.equal(asked.hour, 29_000, `entering the window at ${hour} asks nothing new`);
    assert.equal(asked.tags, windowTags, 'every country of the windows held, a country entering mid-window too');
  }
  assert.equal(chordNameHour(30_517, 29_000), 29_000);
  assert.notEqual(chordNameHour(31_000, 30_990), chordNamesAsked(true, 29_000, 30_240, windowTags, shownTags).hour);
  assert.deepEqual(chordNamesAsked(false, 29_000, 30_240, windowTags, shownTags), { hour: 30_240, tags: shownTags });
  assert.deepEqual(chordNamesAsked(true, null, 30_240, windowTags, shownTags), { hour: 30_240, tags: shownTags });
  assert.equal(chordNameHour(30_240, null), 30_240);
});
