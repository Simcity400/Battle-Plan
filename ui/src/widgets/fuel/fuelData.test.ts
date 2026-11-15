import test from 'node:test';
import assert from 'node:assert/strict';
import { dayAmounts, fuelParts, fuelTooltips, partsStructure, percentFull, stripBars, stripNote, validateCountryFuel, validateFuelAwardHistory, validateFuelAwards } from './fuelData.ts';
import type { CountryFuelReply, FuelLabel, FuelReadAt } from '../../generated/dashboard';

const label = (key: string, template: string | null = `${key}: {NUM}`): FuelLabel => ({ key, template, arguments: ['NUM'], reason: template ? null : 'absent' });
const read = (patch: Partial<FuelReadAt> = {}): FuelReadAt => ({
  readHour: 48, country: 7, countryInstance: 0, fuelRaw: String(80 * 32768), capacityRaw: String(100 * 32768), lendLeaseRaw: '0',
  dayHour: 48, day: { gainedRaw: '500000', burnedRaw: String(3 * 32768), receivedRaw: ['100000', '0', '50000'], otherRaw: '32768', awards: null },
  leftOut: null, awardsReason: 'Applied fuel updates were not recorded', continuityStartHour: null, ...patch });
const reply = (patch: Partial<CountryFuelReply> = {}): CountryFuelReply => ({
  tag: 'ONE', quantityScale: 32768, gainScale: 100000, dayHours: 24, detailHours: 720, consumerNames: ['Army', 'Air', null],
  gainLabels: [label('Base'), label('States'), label('Oil'), label('Other', null)], transferLabels: [label('In'), label('Out'), label('Flat')],
  read: read(), missing: null, consumption: null, consumptionMissing: 'not read', warnings: [], ...patch });
const applied = () => read({ awardsReason: null, continuityStartHour: 0, day: { gainedRaw: '500000', burnedRaw: String(3 * 32768),
  receivedRaw: ['100000', '0', '50000'], otherRaw: String(32768 + 16384), awards: { gainsRaw: ['300000', '150000', '50000', '0'], ownGainRaw: '500000',
    ownGainQuantityRaw: String(5 * 32768), transfersRaw: ['8192', '16384', '32768', String(32768 + 16384)], consumedRaw: String(3 * 32768),
    receivedRaw: ['100000', '0', '50000'] } } });

test('a fuel reply holds a read or says why not, and its day or why it is left out', () => {
  assert.equal(validateCountryFuel(reply()).tag, 'ONE');
  assert.doesNotThrow(() => validateCountryFuel(reply({ read: null, missing: 'No fuel status' })));
  assert.doesNotThrow(() => validateCountryFuel(reply({ read: read({ day: null, leftOut: 'notUpdated' }) })));
  assert.throws(() => validateCountryFuel(reply({ read: null, missing: null })), /malformed/);
  assert.throws(() => validateCountryFuel(reply({ read: read({ day: null, leftOut: null }) })), /malformed/, 'a day without a reason is not allowed');
  assert.throws(() => validateCountryFuel(reply({ read: read({ fuelRaw: '1.5' }) })), /malformed/);
});

test('a day is gained, burned and net, rounded once from the recorded values; an applied day nets its converted gain', () => {
  assert.deepEqual(dayAmounts(reply(), read(), 0), { gained: 5n, burned: 3n, net: 2n, gaining: true });
  assert.deepEqual(dayAmounts(reply(), applied(), 0), { gained: 5n, burned: 3n, net: 2n, gaining: true });
  assert.equal(dayAmounts(reply(), read({ day: null, leftOut: 'overlaps' }), 0), null);
  assert.equal(percentFull(read()), 80n);
  assert.equal(percentFull(read({ capacityRaw: '0' })), null);
});

test('a burned day splits into consumers, transfers, other and the conversion residual, which sum to the total exactly', () => {
  const values = fuelParts(reply(), applied(), 'burn')!;
  assert.deepEqual(values.parts.map(part => part.id), ['consumer-0', 'consumer-1', 'consumer-2', 'outgoing', 'flat', 'other', 'conversion']);
  assert.equal(values.parts.reduce((sum, part) => sum + part.raw, 0n), values.total);
  assert.equal(values.parts.find(part => part.id === 'consumer-2')!.label, 'Other', 'a consumer the content does not name');
  const gain = fuelParts(reply(), applied(), 'gain')!;
  assert.deepEqual(gain.parts.map(part => [part.id, part.raw]), [['gain-0', 300000n], ['gain-1', 150000n], ['gain-2', 50000n], ['gain-3', 0n]]);
  assert.equal(gain.transfer?.raw, 8192n, 'incoming lend-lease apart from own production');
  const ring = fuelParts(reply(), read(), 'gain')!;
  assert.deepEqual([ring.parts.map(part => part.id), ring.reason], [['unidentified'], 'Applied fuel updates were not recorded']);
});

test('a breakdown renders again only when which parts it shows or their names change', () => {
  const before = partsStructure(reply(), 'burn');
  assert.equal(partsStructure(reply({ read: read({ fuelRaw: '1', day: { ...read().day!, burnedRaw: '999999' } }) }), 'burn'), before);
  assert.notEqual(partsStructure(reply({ read: applied() }), 'burn'), before);
  assert.notEqual(partsStructure(reply({ read: read({ day: null, leftOut: 'notUpdated' }) }), 'burn'), before);
});

test('the tooltip says why a day is left out and never shows a zero for it', () => {
  const [full] = fuelTooltips(reply({ read: read({ day: null, leftOut: 'ownershipNotRecorded' }) }), 'Day', { fuel: 'a', capacity: 'b', gained: 'c', burned: 'd' }, 0);
  assert.deepEqual(full.rows.find(row => row.label === 'Gained and burned')?.value, 'Not known');
  const [missing] = fuelTooltips(reply({ read: null, missing: 'No fuel status is recorded' }), 'Day', { fuel: 'a', capacity: 'b', gained: 'c', burned: 'd' }, 0);
  assert.deepEqual(missing.rows.map(row => row.label), ['No fuel status is recorded']);
});

test('the strip has a bar a read, gained up and burned down in fuel', () => {
  const run = (hour: number, value: number) => ({ firstHour: hour, lastHour: hour + 23, value });
  const summary = (runs: ReturnType<typeof run>[]) => ({ from: 0, to: 100, width: 4, buckets: [], runs });
  assert.deepEqual(stripBars(summary([run(24, 200000), run(48, 100000)]), summary([run(24, 65536)]), 24),
    [{ firstHour: 0, lastHour: 24, up: 2, down: 2 }, { firstHour: 24, lastHour: 48, up: 1, down: 0 }]);
});

test('a bin of many reads is a span bar over its hours with each half\'s greatest read, never a bar at a bucket\'s hour', () => {
  // The summary dates its greatest read only to a bucket (here its first hour): the bar spans the bin instead.
  const bin = (first: number, last: number, most: number) => ({ firstHour: first, lastHour: last, firstValue: 0, lastValue: 0, lastValueHour: first,
    minHour: first, minValue: 0, maxHour: first, maxValue: most, firstBreak: false, hasBreak: false });
  const summary = (buckets: ReturnType<typeof bin>[]) => ({ from: 0, to: 191, width: 2, buckets });
  assert.deepEqual(stripBars(summary([bin(0, 95, 200000), bin(96, 191, 100000)]), summary([bin(0, 95, 65536), bin(96, 191, 0)]), 24), [
    { firstHour: -1, lastHour: 95, up: 2, down: 2, span: true }, { firstHour: 95, lastHour: 191, up: 1, down: 0, span: true }]);
});

test('a strip with no bar says why, never a blank band: not recorded with its reason, loading, or a failed read', () => {
  const lines = (patch: Partial<{ landed: boolean; unrecorded: string | null; failure: string | null }> = {}) => ({ landed: true, unrecorded: null, failure: null, ...patch });
  const why = 'FuelDays held no row at these hours';
  assert.equal(stripNote(3, lines({ unrecorded: why }), lines()), null, 'a bar drawn needs no words');
  assert.deepEqual(stripNote(0, lines({ unrecorded: why }), lines({ unrecorded: why })), { text: 'Gain and burn not recorded', reason: why });
  assert.deepEqual(stripNote(0, lines({ landed: false }), lines()), { text: 'Loading gain and burn…', reason: null });
  assert.deepEqual(stripNote(0, lines(), lines({ failure: 'refused' })), { text: 'Gain and burn could not be loaded', reason: 'refused' });
  assert.equal(stripNote(0, lines(), lines())?.text, 'Gain and burn not recorded', 'lines landed with no value in view are not a zero');
});

test('recipients are listed once each, and a recipient history names why a day is unknown', () => {
  const awards = { tag: 'ONE', readHour: 48, country: 7, countryInstance: 0, receivedRaw: ['1', '0', '0'], warnings: [],
    recipients: [{ key: 'a', kind: 0, reference: '9', owner: 7, host: '0', firstHour: 25, lastHour: 48, amountRaw: '1', recordedName: 'Div', nameReason: null }] };
  assert.equal(validateFuelAwards(awards).recipients.length, 1);
  assert.throws(() => validateFuelAwards({ ...awards, recipients: [...awards.recipients, ...awards.recipients] }), /malformed/);
  const history = { tag: 'ONE', country: 7, window: 0, recipientKey: 'a', firstHour: 0, lastHour: 719, warnings: [],
    runs: [{ firstHour: 0, lastHour: 23, startsSegment: true, amountRaw: null, reason: 'not recorded' }, { firstHour: 24, lastHour: 47, startsSegment: false, amountRaw: '5', reason: null }] };
  assert.equal(validateFuelAwardHistory(history).runs.length, 2);
  assert.throws(() => validateFuelAwardHistory({ ...history, runs: [{ ...history.runs[0], reason: null }] }), /malformed/);
});
