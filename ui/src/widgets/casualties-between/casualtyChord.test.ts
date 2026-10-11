import assert from 'node:assert/strict';
import test from 'node:test';
import { casualtyChord, chordPairs, OTHER_ID, pairTooltip } from './casualtyChord.ts';
import type { PairFlow } from './casualtyPairsData';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';

const labels = (tag: string) => ({ name: `Name ${tag}` });
const color = () => '#888888';
test('chord folding retains every exact constituent, signed correction and zero', () => {
  const flows: PairFlow[] = Array.from({ length: 40 }, (_, i) => ({ id: `pair ${i}`, from: 'acting', to: `mod enemy ${i}`,
    value: i === 0 ? -12n : i === 1 ? 0n : 9007199254740993n + BigInt(i), readHour: 100 + i, incomplete: i === 2 }));
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
  const refused = { name: 'Dated acting country', flagReason: 'Flag refused at read 100', flagPending: false };
  const pending = { name: 'Dated enemy', flagPending: true };
  const dated: CountryLabel = tag => tag === 'acting' ? refused : pending;
  const flow: PairFlow = { id: 'correction', from: 'acting', to: 'enemy', value: -9007199254740993n, readHour: 100, incomplete: false };
  const snapshot = { countries: [flow.from, flow.to], flows: [flow] };
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
