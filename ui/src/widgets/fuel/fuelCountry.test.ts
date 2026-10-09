import test from 'node:test';
import assert from 'node:assert/strict';
import { fuelCountryHistory, fuelCountryRead, fuelCountrySpans, fuelCountryToken, readFuelCountry } from './fuelCountry.ts';
import type { FuelCountrySelection } from './fuelCountry';
import type { CountryIdentityHistory, FuelReply } from '../../generated/dashboard';

const selection: FuelCountrySelection = { campaign: 'synthetic', branch: '1', tag: 'X', originOffset: 5 };
const identities: CountryIdentityHistory[] = [
  { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }, { firstHour: 110, lastHour: 120, slot: 2, countryId: 20 }] },
  { tag: 'Y', identities: [{ firstHour: 110, lastHour: 114, slot: 1, countryId: 10 }, { firstHour: 117, lastHour: 120, slot: 3, countryId: 30 }] },
];
const label = { key: 'synthetic', template: '{NUM}', arguments: ['NUM'] as ('NUM' | 'OIL')[], reason: null };
const read = (firstHour: number, lastHour: number, raw: string) => ({ firstHour, lastHour, country: '1', countryInstance: 10,
  startsSegment: true, continuityStartHour: 100, fuelRaw: '0', capacityRaw: '0', lendLeaseRaw: '0', awardsReason: null, leftOut: null,
  day: { gainedRaw: raw, burnedRaw: '0', receivedRaw: ['0', '0', '0'] as [string, string, string], otherRaw: '0', awards: null } });
const reply = (tag: string, reads: FuelReply['reads']): FuelReply => ({ branchId: '1', tag, reads, warnings: [],
  quantityScale: 32768, gainScale: 100000, dayHours: 24, detailHours: 720, consumerNames: [null, null, null],
  gainLabels: [label, label, label, label], transferLabels: [label, label, label] });

test('Fuel country pins store a qualified relative origin and refuse unknown or conflicting native origins', () => {
  assert.deepEqual(readFuelCountry(fuelCountryToken(selection), 'synthetic', '1'), selection);
  assert.equal(readFuelCountry(fuelCountryToken(selection), 'other', '1'), null);
  assert.equal(readFuelCountry(fuelCountryToken(selection), 'synthetic', '2'), null);
  assert.equal(readFuelCountry('malformed', 'synthetic', '1'), null);
  assert.equal(readFuelCountry(fuelCountryToken(selection), 'synthetic', '1', { kind: 'tags', tags: ['Y'] }), null);
  const wide = { ...selection, originOffset: 0xffff_ffff };
  assert.deepEqual(readFuelCountry(fuelCountryToken(wide), 'synthetic', '1'), wide);
  assert.ok(fuelCountrySpans([{ tag: 'X', identities: [{ firstHour: -0x8000_0000, lastHour: 0x7fff_ffff, slot: 1, countryId: 10 }] }],
    wide, -0x8000_0000, 0x7fff_ffff));
  assert.equal(fuelCountrySpans([], selection, 100, 120), null);
  assert.equal(fuelCountrySpans(identities, { ...selection, originOffset: -1 }, 100, 120), null);
  assert.equal(fuelCountrySpans(identities, { ...selection, originOffset: 21 }, 100, 120), null);
  assert.equal(fuelCountrySpans([...identities, { tag: 'conflicting', identities: [{ firstHour: 105, lastHour: 111, slot: 1, countryId: 10 }] }], selection, 100, 120), null);
});

test('Fuel alias plots clip replacements and memberships without changing supplying dates, signed text or zeros', () => {
  const spans = fuelCountrySpans(identities, selection, 100, 120, new Map([
    ['X', [{ firstHour: 104, lastHour: 120 }]], ['Y', [{ firstHour: 111, lastHour: 120 }]],
  ]))!;
  const replies = [reply('X', [read(100, 109, '-1100000'), read(110, 120, '9900000')]),
    reply('Y', [read(110, 114, '0'), read(117, 120, '8800000')])];
  assert.equal(fuelCountryRead(replies, spans, 105)!.read.firstHour, 100);
  assert.equal(fuelCountryRead(replies, spans, 105)!.read.day!.gainedRaw, '-1100000');
  assert.equal(fuelCountryRead(replies, spans, 112)!.reply.tag, 'Y');
  assert.equal(fuelCountryRead(replies, spans, 112)!.read.firstHour, 110);
  assert.equal(fuelCountryRead(replies, spans, 116), null);
  assert.equal(fuelCountryRead(replies, spans, 119), null);
  const history = fuelCountryHistory(replies, spans, 'gain');
  assert.deepEqual(history[0].runs.map(run => [run.firstHour, run.lastHour, run.value]), [[104, 109, 0], [111, 114, 0]]);
  assert.equal(replies[0].reads[0].firstHour, 100);
  assert.equal(replies[1].reads[0].firstHour, 110);
});

test('Fuel authenticates a late entry in a 5,000-country native catalogue without a chosen count limit', () => {
  const many = Array.from({ length: 5000 }, (_, index) => ({ tag: `synthetic:${index}`, identities: [
    { firstHour: 100, lastHour: 120, slot: index, countryId: index + 10000 },
  ] }));
  const spans = fuelCountrySpans(many, { ...selection, tag: 'synthetic:4999' }, 100, 120)!;
  assert.deepEqual([...spans.keys()], ['synthetic:4999']);
  assert.deepEqual(spans.get('synthetic:4999')!.map(span => [span.firstHour, span.lastHour]), [[100, 120]]);
});
