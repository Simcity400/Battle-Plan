import test from 'node:test';
import assert from 'node:assert/strict';
import type { FuelAwardTotals, FuelAwardsReply, FuelAwardHistoryReply, FuelLabel, FuelRead, FuelReply } from '../../generated/dashboard';
import { dayAmounts, formatFuel, formatFuelPart, fuelAwardBasis, fuelChoiceKey, fuelDecimals, fuelHistoryRow, fuelLabel, fuelPartHistory, fuelParts, fuelPartShare, fuelRecipientAmount, fuelRuns, fuelTooltips, fuelView, percentFull, validateFuelReply, validateFuelAwardsReply, validateFuelAwardHistory } from './fuelData.ts';
import { OPTIONS } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';

const Q = 32_768;
const label = (key: string): FuelLabel => ({ key, template: `${key}: {NUM}`, arguments: ['NUM'], reason: null });

test('fuel candidate ownership changes with comparisons, membership policy/spans and visible capacity', () => {
  const countries = { tags: ['GER', 'ITA'], membership: null };
  const first = fuelChoiceKey({ kind: 'tags', tags: countries.tags }, countries, true);
  assert.equal(first, fuelChoiceKey({ kind: 'tags', tags: ['GER', 'ITA'] }, { ...countries, tags: ['GER', 'ITA'] }, true));
  assert.notEqual(first, fuelChoiceKey({ kind: 'tags', tags: ['ENG', 'FRA'] }, { tags: ['ENG', 'FRA'], membership: null }, true));
  assert.notEqual(first, fuelChoiceKey({ kind: 'tags', tags: countries.tags }, countries, false));
  const group = { kind: 'group' as const, group: { type: 'humans' as const }, membership: 'hour' as const, aggregate: false };
  const member = { ...countries, membership: new Map([['GER', [{ tag: 'GER', firstHour: 0, lastHour: 50 }]]]) };
  const hourly = fuelChoiceKey(group, member, true);
  assert.notEqual(hourly, fuelChoiceKey({ ...group, membership: 'ever' }, { ...countries, membership: null }, true));
  assert.notEqual(hourly, fuelChoiceKey(group, { ...member, membership: new Map([['GER', [{ tag: 'GER', firstHour: 0, lastHour: 60 }]]]) }, true));
});
const read = (firstHour: number, lastHour: number, fuel: number, capacity: number, day: [number, number] | 'overlaps' | 'notUpdated' | 'ownershipNotRecorded', startsSegment = false): FuelRead => ({
  country: '7', countryInstance: null, continuityStartHour: null, awardsReason: 'Applied fuel source and recipient history not recorded',
  firstHour, lastHour, startsSegment, fuelRaw: String(fuel), capacityRaw: String(capacity), lendLeaseRaw: '0',
  day: Array.isArray(day) ? { gainedRaw: String(day[0]), burnedRaw: String(day[1]), receivedRaw: ['0', '0', '0'], otherRaw: '0', awards: null } : null,
  leftOut: Array.isArray(day) ? null : day,
});
// Day 1: 10 fuel of 20, gained 3.5, burned 1.25. A refresh two hours later overlaps it.
// Day 2 (after a gap): full at 20, gained 2, burned 0. Then a read after a day without states.
const REPLY: FuelReply = {
  branchId: '1', tag: 'GER', quantityScale: Q, gainScale: 100_000, dayHours: 24, detailHours: 720, warnings: [],
  gainLabels: [label('base'), label('states'), label('oil'), label('negative-flat')],
  transferLabels: [label('incoming'), label('outgoing'), label('flat')],
  consumerNames: ['Land', 'Air', 'Sea'],
  reads: [
    read(1017, 1018, 10 * Q, 20 * Q, [350_000, 1.25 * Q], true),
    read(1019, 1040, 10.5 * Q, 20 * Q, 'overlaps'),
    { ...read(1065, 1070, 20 * Q, 20 * Q, [200_000, 0], true), lendLeaseRaw: String(1.5 * Q) },
    { ...read(1089, 1089, 7 * Q, 20 * Q, 'notUpdated'), lendLeaseRaw: String(2 * Q) },
    read(1113, 1113, 7 * Q, 20 * Q, 'ownershipNotRecorded'),
  ],
};
const COLORS = { fuel: 'blue', capacity: 'grey', gained: 'azure', burned: 'orange' };
const forms = (hour: number, digits = 2) => {
  const view = fuelView(REPLY, 0, 5000);
  return fuelTooltips(REPLY, view.reads, hour, `Hour ${hour}`, COLORS, digits);
};
const tip = (hour: number, digits = 2) => forms(hour, digits)[0];
/** The decimals a widget with these stored settings writes. */
const chosen = (stored: Record<string, unknown>) => fuelDecimals(resolveOptions(OPTIONS, stored as never));

test('burn parts preserve fixed5 receipts and the signed tank-conversion residual', () => {
  const sample = read(1, 1, 10, 20, [17, 32769]);
  sample.day!.receivedRaw = ['50001', '25000', '24999'];
  sample.day!.otherRaw = '1';
  const burn = fuelParts(REPLY, sample, 'burn')!;
  assert.equal(burn.scale, 100000n * 32768n);
  assert.equal(burn.total, 32769n * 100000n);
  assert.deepEqual(burn.parts.map(part => part.raw), [50001n * 32768n, 25000n * 32768n, 24999n * 32768n, 100000n, 0n]);
  sample.day!.burnedRaw = '32768';
  const truncated = fuelParts(REPLY, sample, 'burn')!;
  assert.equal(truncated.parts.at(-1)!.raw, -100000n);
  assert.equal(truncated.parts.reduce((sum, part) => sum + part.raw, 0n), truncated.total);
  assert.equal(fuelParts(REPLY, sample, 'gain')!.parts[0].raw, 17n);
  assert.equal(fuelParts(REPLY, REPLY.reads[1], 'burn'), null);
});

test('the reply must have its reads in order, each with its day or the reason it is left out', () => {
  assert.equal(validateFuelReply(REPLY), REPLY);
  const bad = (change: (reply: FuelReply) => unknown) => {
    const copy = structuredClone(REPLY);
    change(copy);
    assert.throws(() => validateFuelReply(copy), /malformed/);
  };
  bad(reply => { reply.reads[1].firstHour = 1018; });
  bad(reply => { (reply.reads[0].day as unknown as { receivedRaw: unknown[] }).receivedRaw = [1, '0', '0']; });
  bad(reply => { (reply.reads[0].day as unknown as { receivedRaw: unknown[] }).receivedRaw = ['0', '0']; });
  bad(reply => { reply.consumerNames = [null, null, '']; delete (reply as Partial<FuelReply>).consumerNames; });
  bad(reply => { reply.reads[0].leftOut = 'overlaps'; });
  bad(reply => { reply.reads[1].leftOut = null; });
  bad(reply => { reply.reads[0].fuelRaw = '1.5'; });
  bad(reply => { (reply.reads[0] as { lendLeaseRaw?: string }).lendLeaseRaw = undefined; });
  bad(reply => { reply.quantityScale = 0; });
  bad(reply => { (reply.reads[1] as { leftOut: string }).leftOut = 'later'; });
  bad(reply => { (reply.reads[1] as { leftOut: string }).leftOut = 'toString'; });
});

test('daily raw strings accept 24 native i64 updates and exact signed i128 boundaries without clipping', () => {
  const max = (1n << 127n) - 1n;
  const min = -max - 1n;
  const nativeMaxDay = 24n * ((1n << 63n) - 1n);
  const nativeMinDay = 24n * -(1n << 63n);
  assert.equal(nativeMaxDay.toString(), '221360928884514619368');
  for (const raw of [nativeMaxDay, nativeMinDay, max, min]) {
    const copy = structuredClone(REPLY);
    const day = copy.reads[0].day!;
    day.gainedRaw = raw.toString(); day.burnedRaw = raw.toString(); day.otherRaw = raw.toString();
    day.receivedRaw = [raw.toString(), '0', '0'];
    assert.equal(validateFuelReply(copy), copy);
    assert.equal(BigInt(day.gainedRaw), raw);
  }
  for (const raw of [max + 1n, min - 1n]) {
    const copy = structuredClone(REPLY);
    copy.reads[0].day!.gainedRaw = raw.toString();
    assert.throws(() => validateFuelReply(copy), /malformed/);
  }
  // Decimal wire fields remain strings, even when their numeric value is small.
  const malformed = structuredClone(REPLY);
  (malformed.reads[0].day as unknown as { gainedRaw: unknown }).gainedRaw = 1;
  assert.throws(() => validateFuelReply(malformed), /malformed/);
});

test('a date range cuts the reads, keeps every read hour for dating, and keeps only the days read inside it', () => {
  const view = fuelView(REPLY, 1030, 1067);
  assert.deepEqual(view.reads.map(item => [item.firstHour, item.lastHour, item.startsSegment]), [[1030, 1040, false], [1065, 1067, true]]);
  assert.deepEqual(view.readHours, [1017, 1019, 1065, 1089, 1113]);
  // Day 2's bar is cut to the range's start; day 1 was read before it.
  assert.deepEqual(view.bars, [{ firstHour: 1041, lastHour: 1065, up: 2, down: 0 }]);
  assert.deepEqual(fuelView(REPLY, 0, 5000).bars, [
    { firstHour: 993, lastHour: 1017, up: 3.5, down: 1.25 },
    { firstHour: 1041, lastHour: 1065, up: 2, down: 0 },
  ]);
  const runs = fuelRuns(REPLY.reads, Q, 'fuel');
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.value, run.breakBefore]),
    [[1017, 1018, 10, true], [1019, 1040, 10.5, false], [1065, 1070, 20, true], [1089, 1089, 7, false], [1113, 1113, 7, false]]);
  assert.deepEqual(fuelRuns(REPLY.reads, Q, 'capacity').map(run => run.value), [20, 20, 20, 20, 20]);
});

test('amounts are exact: each part and the net are rounded once from the recorded values', () => {
  // Gained 0.005 and burned 1/32768 (0.0000305…): the net rounds from 0.0049694…
  assert.deepEqual(dayAmounts(REPLY, read(1, 1, 0, Q, [500, 1]), 2), { gained: 1n, burned: 0n, net: 0n, gaining: true });
  assert.deepEqual(dayAmounts(REPLY, REPLY.reads[0], 2), { gained: 350n, burned: 125n, net: 225n, gaining: true });
  assert.equal(dayAmounts(REPLY, REPLY.reads[1], 2), null);
  assert.equal(formatFuel(String(1_234_567.5 * Q), Q, 0), '1,234,568');
  assert.equal(formatFuel('-16384', Q, 2), '−0.5');
  assert.equal(formatFuel(String(20 * Q), Q, 2), '20');
  assert.equal(percentFull(REPLY.reads[1]), 53n);
  assert.equal(percentFull(read(1, 1, 5, 0, 'overlaps')), null);
});

test('the tooltip lists the tank, then the day before its read, and dates a carried read', () => {
  const day = tip(1018);
  assert.deepEqual(day.rows.map(row => [row.label, row.value]), [
    ['Fuel', '10'], ['Capacity', '20'], ['Full', '50%'], ['Gained (own production)', '+3.5'], ['Burned', '−1.25'], ['Net', '+2.25']]);
  assert.equal(day.rows[5].tone, 'positive');
  assert.match(day.note!, /^Previous 24 hours; excludes incoming lend-lease\.$/);
  // Without lend-lease at the read nothing more is said; with it, its last hour is named, never a day's total.
  assert.doesNotMatch(day.note!, /arriving/);
  assert.match(tip(1065).note!, /Daily lend-lease total unavailable/);
  assert.match(tip(1017).note!, /^Previous 24 hours/);
  // A full tank says why the level did not rise by the net.
  assert.match(tip(1065).note!, /discards fuel beyond its capacity/);
  const overlapping = tip(1019);
  assert.deepEqual(overlapping.rows.at(-1), { label: 'Gained and burned', value: 'Counted in another day', tone: 'muted', divider: true });
  assert.match(overlapping.note!, /Included in another day/);
  assert.match(tip(1089).note!, /no states were owned/);
  // A day the game did not update makes no claim from its frozen lend-lease value.
  assert.doesNotMatch(tip(1089).note!, /lend-lease/i);
  // Unknown ownership is its own reason, never a bar.
  assert.equal(tip(1113).rows.at(-1)!.value, 'Not known');
  assert.match(tip(1113).note!, /Daily data incomplete/);
  assert.deepEqual(tip(1050).rows, [{ label: 'Not recorded at this time', tone: 'muted' }]);
});

test('fuel amounts are whole numbers by default, and the settings ask for more decimals', () => {
  assert.equal(chosen({}), 0);
  assert.equal(chosen({ fuelDecimals: 2 }), 2);
  assert.equal(chosen({ fuelDecimals: 5 }), 5);
  // A stored value out of range keeps the default rather than inventing digits.
  assert.equal(chosen({ fuelDecimals: 9 }), 0);
  assert.equal(chosen({ fuelDecimals: 1.5 }), 0);
  // Gained 3.5 and burned 1.25 each round once; the net rounds from the exact 2.25, not from 4 − 1.
  const rows = (digits: number) => tip(1018, digits).rows.map(row => [row.label, row.value]);
  assert.deepEqual(rows(chosen({})), [
    ['Fuel', '10'], ['Capacity', '20'], ['Full', '50%'], ['Gained (own production)', '+4'], ['Burned', '−1'], ['Net', '+2']]);
  assert.deepEqual(rows(chosen({ fuelDecimals: 1 })), [
    ['Fuel', '10'], ['Capacity', '20'], ['Full', '50%'], ['Gained (own production)', '+3.5'], ['Burned', '−1.3'], ['Net', '+2.3']]);
  assert.equal(formatFuel(String(371_500.25 * Q), Q, chosen({})), '371,500');
  assert.equal(formatFuel(String(371_500.5 * Q), Q, chosen({ fuelDecimals: 3 })), '371,500.5');
  // A net that rounds to nothing is written 0 without a tone, while a full tank's
  // exact gain still explains the discarded fuel.
  const full = { ...REPLY, reads: [read(1017, 1018, 20 * Q, 20 * Q, [40_000, 0.25 * Q])] };
  const small = fuelTooltips(full, full.reads, 1018, 'Day', COLORS, 0)[0];
  assert.deepEqual(small.rows.at(-1), { label: 'Net', value: '0', strong: true, tone: undefined });
  assert.match(small.note!, /discards fuel beyond its capacity/);
});

test('the compact tooltip form keeps the tank level and the net', () => {
  assert.deepEqual(forms(1018)[1].rows.map(row => row.label), ['Fuel', 'Net']);
});

const awards = (): FuelAwardTotals => ({ gainsRaw: ['400000', '-50000', '0', '17'], ownGainRaw: '350017',
  ownGainQuantityRaw: '114695', transfersRaw: ['8000', '100', '200', '303'], consumedRaw: '32769', receivedRaw: ['50001', '25000', '24999'] });
const actual = (firstHour = 100, lastHour = 123): FuelRead => {
  const totals = awards();
  return { ...read(firstHour, lastHour, Q, 2 * Q, [0, 0]), countryInstance: 9, continuityStartHour: 77, awardsReason: null,
    day: { gainedRaw: totals.ownGainRaw, burnedRaw: totals.consumedRaw, receivedRaw: totals.receivedRaw, otherRaw: totals.transfersRaw[3], awards: totals } };
};

test('actual sources, transfers and per-hour quantity operands stay exact without counting Other twice', () => {
  const source = actual(), gain = fuelParts(REPLY, source, 'gain')!, burn = fuelParts(REPLY, source, 'burn')!;
  assert.deepEqual(gain.parts.map(part => part.raw), [400000n, -50000n, 0n, 17n]);
  assert.equal(gain.parts.reduce((sum, part) => sum + part.raw, 0n), gain.total);
  assert.equal(gain.transfer!.raw, 8000n);
  assert.equal(gain.total, 350017n); // Incoming LL is outside this own-production denominator.
  assert.deepEqual(burn.parts.slice(3, 6).map(part => part.raw), [10000000n, 20000000n, 300000n]);
  assert.equal(burn.parts.reduce((sum, part) => sum + part.raw, 0n), burn.total);
  assert.equal(burn.parts.at(-1)!.raw, (32769n - 303n) * 100000n - 100000n * 32768n);
  assert.equal(dayAmounts(REPLY, source, 2)!.net, 250n);
  const rounded = actual();
  Object.assign(rounded.day!, { gainedRaw: '500', burnedRaw: '0', receivedRaw: ['0', '0', '0'], otherRaw: '0' });
  Object.assign(rounded.day!.awards!, { ownGainRaw: '500', ownGainQuantityRaw: '151', consumedRaw: '0' });
  // 23 hourly raw20 conversions + one raw40 give151 Q15, while converting their raw500 sum gives163.
  assert.deepEqual(dayAmounts(REPLY, rounded, 2), { gained: 1n, burned: 0n, net: 0n, gaining: true });
  const conversion = burn.parts.at(-1)!;
  assert.equal(formatFuelPart(conversion.raw, burn.scale), '\u22120.00921630859375');
  assert.equal(formatFuelPart(conversion.raw, burn.scale, 5), '\u22120.00922');
  assert.equal(formatFuelPart(conversion.raw, burn.scale, 0), '0');
  assert.equal(fuelPartShare(-1n, 3n), '\u221233.3%');
  assert.equal(fuelPartShare(-1n, -3n), '33.3%');
  assert.equal(fuelPartShare(0n, 0n), '\u2014');
});

test('written decimals change only the text: parts, totals and shares stay exact', () => {
  const source = actual();
  for (const measure of ['gain', 'burn'] as const) {
    const exact = fuelParts(REPLY, source, measure)!;
    for (const digits of [0, 2, 5]) {
      const written = fuelParts(REPLY, source, measure, REPLY, digits)!;
      assert.deepEqual(written.parts.map(part => part.raw), exact.parts.map(part => part.raw));
      assert.equal(written.total, exact.total);
      assert.equal(written.parts.reduce((sum, part) => sum + part.raw, 0n), written.total);
      assert.deepEqual(written.parts.map(part => fuelPartShare(part.raw, written.total)), exact.parts.map(part => fuelPartShare(part.raw, exact.total)));
    }
  }
  const gain = fuelParts(REPLY, source, 'gain', REPLY, 0)!;
  // 3.50017 fuel gained; the states source takes away 0.5.
  assert.equal(formatFuelPart(gain.total, gain.scale, 0), '4');
  assert.equal(formatFuelPart(gain.total, gain.scale, 2), '3.5');
  assert.equal(formatFuelPart(gain.total, gain.scale), '3.50017');
  assert.deepEqual(gain.parts.map(part => part.label), ['base: 4', 'states: \u22121', 'oil: 0', 'negative-flat: 0']);
  assert.deepEqual(fuelParts(REPLY, source, 'gain', REPLY, 5)!.parts.map(part => part.label), ['base: 4', 'states: \u22120.5', 'oil: 0', 'negative-flat: 0.00017']);
  assert.equal(fuelPartHistory({ ...REPLY, reads: [source] }, 'gain', REPLY, 0).find(part => part.id === 'gain-1')!.label, 'states: \u22121');
  assert.ok(fuelTooltips({ ...REPLY, reads: [source] }, [source], 123, 'Day', COLORS, 0)[0].rows
    .some(row => row.label === 'Lend-lease received (separate)' && row.value === '0'));
});

test('the exact supplying day resolves native template NUM; uncaptured historical OIL is never a later count', () => {
  assert.equal(fuelLabel({ key: 'synthetic-oil', template: '{OIL} oil gives {NUM}', arguments: ['OIL', 'NUM'],
    reason: 'Historical OIL operand not recorded' }, 350017n, 100000n), '? oil gives 3.50017');
  assert.equal(fuelLabel({ key: 'synthetic-oil', template: '{OIL} oil gives {NUM}', arguments: ['OIL', 'NUM'],
    reason: 'Historical OIL operand not recorded' }, 350017n, 100000n, 0), '? oil gives 4');
  assert.equal(fuelLabel({ key: 'missing-mod-content', template: null, arguments: ['NUM'], reason: 'Missing key' }, 3n, 1n), null);
});

test('source histories keep actual source keys and signed rows across legacy/refused days without false zero attribution', () => {
  const legacy = read(76, 99, 0, Q, [1, 0]), first = actual(), refused = read(124, 147, 0, Q, [1, 0]), next = actual(148, 171);
  const reply = { ...REPLY, reads: [legacy, first, refused, next] };
  const series = fuelPartHistory(reply, 'gain');
  assert.deepEqual(series.find(part => part.id === 'unidentified')!.runs.map(run => run.firstHour), [76, 124]);
  assert.deepEqual(series.find(part => part.id === 'gain-0')!.runs.map(run => [run.firstHour, run.breakBefore]), [[100, true], [148, true]]);
  assert.deepEqual(series.find(part => part.id === 'gain-1')!.runs.map(run => run.value), [0, 0]);
  assert.equal(fuelParts(reply, first, 'gain')!.parts[1].raw, -50000n);
  assert.equal(series.find(part => part.id === 'incoming')!.runs[0].value, 8000 / Q);
  assert.equal(fuelAwardBasis(reply, first)!.hour, 100);
  assert.equal(fuelAwardBasis(reply, legacy), null);
  assert.equal(validateFuelReply(reply), reply);
  const missing = structuredClone(reply); delete (missing.reads[1].day as Partial<NonNullable<FuelRead['day']>>).awards;
  assert.throws(() => validateFuelReply(missing), /malformed/);
  const disagreement = structuredClone(reply); disagreement.reads[1].day!.awards!.ownGainRaw = '999';
  assert.throws(() => validateFuelReply(disagreement), /malformed/);
});

test('actual daily lend-lease tooltip replaces only the legacy last-hour claim and keeps the compact own net', () => {
  const source = actual(), reply = { ...REPLY, reads: [source] };
  const full = fuelTooltips(reply, reply.reads, 123, 'Day', COLORS, 2);
  assert.ok(full[0].rows.some(row => row.label === 'Lend-lease received (separate)' && row.value === '+0.24'));
  assert.equal(full[1].rows[1].label, 'Net');
  assert.ok(!full[0].note!.includes('last hour'));
});

test('lazy actual recipient replies retain 5000 signed/zero rows, full native limbs and refused names', () => {
  const reply: FuelAwardsReply = { branchId: '1', tag: 'GER', hour: 100, warnings: ['Names refused: 1'],
    awards: { totals: awards(), recipients: Array.from({ length: 5000 }, (_, i) => ({ key: `opaque-${i}`, kind: i % 3,
      reference: String((1n << 63n) + BigInt(i)), owner: '7', host: i === 1 ? String((1n << 63n) + 1n) : '8', firstHour: 77, lastHour: 100,
      amountRaw: String(i - 2500), recordedName: i === 0 ? null : `Synthetic recipient ${i}`, nameReason: i === 0 ? 'Language mismatch' : null })) } };
  assert.equal(validateFuelAwardsReply(reply), reply);
  assert.equal(reply.awards.recipients.length, 5000);
  assert.equal(reply.awards.recipients[1].kind, 1);
  assert.equal(reply.awards.recipients[1].host, '9223372036854775809');
  const invalidHost = structuredClone(reply); invalidHost.awards.recipients[1].host = String(1n << 64n);
  assert.throws(() => validateFuelAwardsReply(invalidHost), /malformed/);
  assert.equal(reply.awards.recipients[0].amountRaw, '-2500');
  const missing = structuredClone(reply); missing.awards.recipients[1].key = missing.awards.recipients[0].key;
  assert.throws(() => validateFuelAwardsReply(missing), /malformed/);
});

test('selected recipient detail keeps known zero separate from unknown identity/gap windows', () => {
  const reply: FuelAwardHistoryReply = { branchId: '1', tag: 'GER', hour: 100, country: '7', countryInstance: 9,
    continuityStartHour: 77, window: 0, recipientKey: 'opaque', firstHour: 0, lastHour: 719, warnings: [], runs: [
      { firstHour: 100, lastHour: 123, startsSegment: true, amountRaw: '-1', reason: null },
      { firstHour: 124, lastHour: 147, startsSegment: false, amountRaw: null, reason: 'Recipient census not recorded' },
      { firstHour: 148, lastHour: 171, startsSegment: true, amountRaw: '0', reason: null },
    ] };
  assert.equal(validateFuelAwardHistory(reply), reply);
  const inventedZero = structuredClone(reply); inventedZero.runs[1].amountRaw = '0';
  assert.throws(() => validateFuelAwardHistory(inventedZero), /malformed/);
});

test('exact recipient table and chart missing-text seam refuse numeric amounts outside hourly group membership', () => {
  const runs = [{ firstHour: 148, lastHour: 171, startsSegment: true, amountRaw: '123456', reason: null }];
  const within = [{ firstHour: 100, lastHour: 123 }];
  const current = fuelRecipientAmount(runs, 150, within, 100000);
  assert.deepEqual(current, { raw: null, value: null, text: 'Outside the selected group membership' });
  assert.deepEqual(fuelRecipientAmount(runs, 150, null, 100000), { raw: 123456n, value: 1.23456, text: '1.23456' });
});

test('an absent non-transfer source does not inherit incoming lend-lease in the actual history label/amount seam', () => {
  const values = fuelParts(REPLY, actual(), 'gain')!;
  assert.ok(values.transfer);
  assert.equal(fuelHistoryRow(values, 'unidentified', false), null);
  assert.equal(fuelHistoryRow(values, 'incoming', true), values.transfer);
  assert.equal(fuelHistoryRow(values, 'gain-0', false), values.parts[0]);
});

test('recipient history shows a short status instead of backend refusal text', () => {
  const runs = [{ firstHour: 100, lastHour: 123, startsSegment: true, amountRaw: null, reason: 'Native recipient census refused' }];
  assert.deepEqual(fuelRecipientAmount(runs, 100, null, 100000), { raw: null, value: null, text: 'History unavailable' });
});
