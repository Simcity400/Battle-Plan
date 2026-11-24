import { countryKey } from './countryKey.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { airLossParts, ledgerLabel, partAmount, ledgerComplete, ledgerCountries, ledgerCountryAt, ledgerHeaderTag, ledgerPinnedIdentity, ledgerRuns, ledgerTotals, militaryAirAt, militaryAirIndex, militaryAirLineTooltip, validateMilitaryAirReply } from './militaryAirLedger.ts';
import type { MilitaryAirPart } from '../generated/dashboard';
import type { MilitaryAirLedger } from './militaryAirLedger';

const span = (firstHour: number, lastHour = firstHour) => ({ firstHour, lastHour });
type Point = [number, number, string];
interface Leaf { tag: string | null; identity: string | null; dealt: boolean; opponent: string | null; cause: number; points: Point[] }
const leaf = (extra: Partial<Leaf> = {}): Leaf => ({ tag: 'mod country', identity: '3:9', dealt: false, opponent: null, cause: 1,
  points: [[9, 10, '300000'], [8, 11, '200000']], ...extra });
const part = (item: Leaf): MilitaryAirPart => ({ tag: item.tag, identity: item.identity, dealt: item.dealt, cause: item.cause,
  opponentKnown: item.opponent !== null, buildingToken: null, buildingId: null, state: null, points: item.points });
const reply = ({ leaves = [leaf()], ...extra }: Partial<MilitaryAirLedger> & { leaves?: Leaf[] } = {}): MilitaryAirLedger => ({ family: 'airLosses', firstHour: 10, lastHour: 14,
  available: true, observed: [span(10, 11), span(13, 14)], incomplete: [],
  countries: [{ tag: 'mod country', identity: '3:9', present: [span(10, 11), span(13, 14)], membership: null }],
  parts: leaves.map(part), regions: [{ id: '40001', name: null, missing: 'The recorded mod did not define this region.' }], buildings: [], warnings: [], ...extra });

test('ledger totals are exact, summed in numbers while safe and in bigints past that', () => {
  const reference = (leaves: readonly Leaf[]) => {
    const deltas = new Map<number, { dealt: bigint; taken: bigint; nativeHour: number }>();
    for (const item of leaves) {
      let before = 0n;
      for (const [hour, readHour, totalRaw] of item.points) {
        const current = BigInt(totalRaw), delta = current - before;
        before = current;
        const row = deltas.get(readHour) ?? { dealt: 0n, taken: 0n, nativeHour: hour };
        row[item.dealt ? 'dealt' : 'taken'] += delta;
        row.nativeHour = Math.max(row.nativeHour, hour);
        deltas.set(readHour, row);
      }
    }
    let dealt = 0n, taken = 0n;
    return [...deltas].sort(([a], [b]) => a - b).map(([readHour, delta]) => {
      dealt += delta.dealt; taken += delta.taken;
      return { readHour, nativeHour: delta.nativeHour, dealt, taken };
    });
  };
  const points = (...totals: string[]) => totals.map((totalRaw, index): [number, number, string] => [20 - index, 10 + index, totalRaw]);
  for (const leaves of [
    [leaf({ points: points('300000', '200000', '-5') }), leaf({ dealt: true, points: points('7', '7', '900') })],
    [leaf({ points: points('9007199254740993', '1') })],
    [leaf({ points: points('999999999999999') }), leaf({ points: points('999999999999999') }), leaf({ points: points('999999999999999') }),
      leaf({ points: points('999999999999999') }), leaf({ points: points('999999999999999') }), leaf({ points: points('999999999999999') }),
      leaf({ points: points('999999999999999') }), leaf({ points: points('999999999999999') }), leaf({ points: points('999999999999999') }),
      leaf({ points: points('999999999999999') })],
  ]) assert.deepEqual(ledgerTotals(leaves.map(part)), reference(leaves));
});

test('the product validator returns the payload and accepts later publications with earlier native dates', () => {
  const data = reply();
  assert.equal(validateMilitaryAirReply(data), data);
  assert.equal(partAmount(data.parts[0], 10, 10), 300000n);
  assert.equal(partAmount(data.parts[0], 10, 11), 200000n);
  const rangeLeaf = part(leaf({ points: [[11, 12, '-100000']] }));
  assert.equal(partAmount(rangeLeaf, 11, 11), 0n);
  assert.equal(partAmount(rangeLeaf, 11, 12), -100000n);
  assert.throws(() => validateMilitaryAirReply(reply({ leaves: [leaf({ points: [[12, 11, '1']] })] })), /Invalid/);
  assert.throws(() => validateMilitaryAirReply(reply({ leaves: [leaf(), leaf()] })), /Invalid/);
});

test('empty complete reads certify zero, while gaps and unknown bombing callers do not', () => {
  const index = militaryAirIndex(reply({ leaves: [] }));
  assert.equal(militaryAirAt(index, 10, 10)[0].taken, 0n);
  assert.equal(militaryAirAt(index, 10, 10)[0].complete, true);
  assert.deepEqual(militaryAirAt(index, 10, 12), []);
  assert.equal(militaryAirAt(index, 10, 13)[0].complete, false);
  assert.equal(ledgerComplete([span(10, 14)], 10, 13, [span(11)]), false);
  assert.deepEqual(ledgerRuns(index.series[0], 10, 14, false, 100000).map(run => [run.firstHour, run.lastHour, run.value, run.breakBefore]),
    [[10, 11, 0, true], [13, 14, 0, true]]);
});

test('shared indices retain wide signed corrections exactly and name operational losses separately from unknown opponents', () => {
  const wide = (1n << 80n).toString();
  const data = reply({ leaves: [leaf({ points: [[10, 10, wide]] }),
    leaf({ cause: 3, points: [[10, 10, '-100000']] }),
    leaf({ dealt: true, opponent: 'other mod', points: [[10, 10, '700000']] })] });
  const index = militaryAirIndex(validateMilitaryAirReply(data));
  assert.equal(militaryAirIndex(data), index);
  const row = militaryAirAt(index, 10, 10)[0];
  assert.equal(row.taken, BigInt(wide) - 100000n);
  assert.equal(row.dealt, 700000n);
  assert.equal(airLossParts(row, 10, 10).operational, -100000n);
  assert.equal(airLossParts(row, 10, 10).unknownOpponent, BigInt(wide));
});

test('country aliases share their holder total and top memberships clip only the supplied hours', () => {
  const data = reply({ observed: [span(10, 14)], countries: [
    { tag: 'mod country', identity: '3:9', present: [span(10)], membership: [span(10)] },
    { tag: 'renamed', identity: '3:9', present: [span(11, 14)], membership: [span(13)] },
    { tag: 'mod country', identity: '8:99', present: [span(13, 14)], membership: [] },
  ], leaves: [leaf({ points: [[9, 10, '300000']] })] });
  const index = militaryAirIndex(validateMilitaryAirReply(data));
  assert.equal(index.series[0].points, index.series[1].points);
  assert.equal(ledgerCountryAt(index, 'mod country', 10, 11)?.taken, 300000n);
  const rows = militaryAirAt(index, 10, 13);
  assert.equal(rows.find(row => row.identity === '8:99')?.taken, 0n);
  assert.equal(ledgerCountryAt(index, 'mod country', 10, 13, '3:9')?.tag, 'renamed');
  assert.equal(ledgerCountryAt(index, 'mod country', 10, 13, '3:9')?.taken, 300000n);
  assert.equal(ledgerCountryAt(index, 'mod country', 10, 13)?.identity, '3:9');
  assert.equal(ledgerCountryAt(index, 'mod country', 10, 13)?.tag, 'renamed');
  const pinned = JSON.stringify(['campaign', '1', 'mod country', '8:99']);
  const pin = ledgerPinnedIdentity(pinned, 'campaign', '1', 'mod country');
  assert.equal(ledgerCountryAt(index, 'mod country', 10, 13, pin)?.identity, '8:99');
  assert.equal(ledgerPinnedIdentity(pinned, 'another campaign', '1', 'mod country'), undefined);
  assert.equal(ledgerPinnedIdentity(pinned, 'campaign', '2', 'mod country'), undefined);
  assert.equal(ledgerPinnedIdentity(pinned, 'campaign', '1', 'another tag'), undefined);
  assert.equal(ledgerPinnedIdentity('malformed', 'campaign', '1', 'mod country'), undefined);
  assert.deepEqual(ledgerCountries(rows, { kind: 'group', group: { type: 'top', count: 1 }, membership: 'hour', aggregate: false }, [], undefined, 13, 14, 'taken', data.countries).rows.map(row => row.tag), ['renamed']);
  assert.deepEqual(ledgerRuns(index.series[1], 10, 14, false, 100000, index.series[1].country.membership).map(run => [run.firstHour, run.lastHour, run.value]), [[13, 13, 3]]);
  assert.equal(ledgerCountryAt(index, 'mod country', 10, 9), null);
  assert.equal(ledgerHeaderTag(index, null, 'mod country', 9, undefined, null), 'mod country');
  assert.equal(ledgerHeaderTag(index, null, 'mod country', 9, '8:99', { tag: 'mod country', identity: '8:99' }), null, 'a pinned holder without a row is not named by its tag');
  assert.equal(ledgerHeaderTag(index, ledgerCountryAt(index, 'mod country', 10, 10), 'mod country', 10, undefined, null), 'mod country');
  assert.equal(ledgerHeaderTag(index, ledgerCountryAt(index, 'mod country', 10, 13), 'mod country', 13, undefined, null), 'renamed', 'the holder shown, under its tag at the hour');
  assert.equal(ledgerHeaderTag(index, ledgerCountryAt(index, 'mod country', 10, 13, pin), 'mod country', 13, pin, { tag: 'mod country', identity: '8:99' }), 'mod country');
});

test('manual ledger roots include all their native holders without recursively selecting alias holders', () => {
  const data = reply({ observed: [span(10, 14)], countries: [
    { tag: 'X', identity: '3:9', present: [span(10)], membership: null },
    { tag: 'X', identity: '4:10', present: [span(14)], membership: null },
    { tag: 'Y', identity: '8:99', present: [span(10, 11)], membership: null },
    { tag: 'Y', identity: '3:9', present: [span(12), span(14)], membership: null },
    { tag: 'B', identity: '4:10', present: [span(10, 13)], membership: null },
  ], leaves: [leaf({ tag: 'X', points: [[9, 10, '-300000']] }),
    leaf({ tag: 'Y', identity: '8:99', points: [[9, 10, '9900000']] }),
    leaf({ tag: 'B', identity: '4:10', points: [[9, 10, '200000']] })] });
  const index = militaryAirIndex(validateMilitaryAirReply(data));
  const select = (hour: number) => ledgerCountries(militaryAirAt(index, 10, hour), { kind: 'tags', tags: ['X'] }, [], undefined,
    hour, 14, 'taken', data.countries);
  assert.deepEqual(new Set(select(10).rows.map(row => row.identity)), new Set(['3:9', '4:10']));
  assert.equal(select(10).rows.some(row => row.identity === '8:99'), false);
  assert.deepEqual(select(12).rows.map(row => [row.tag, row.identity, row.taken]), [['B', '4:10', 200000n], ['Y', '3:9', -300000n]]);
  assert.deepEqual(select(13).rows.map(row => row.identity), ['4:10'], 'the original holder has no value in its native presence gap');
  assert.deepEqual(new Set(ledgerCountries(militaryAirAt(index, 10, 12), { kind: 'selected' }, ['X'], undefined,
    12, 14, 'taken', data.countries).rows.map(row => row.identity)), new Set(['3:9', '4:10']));
  assert.deepEqual(new Set(ledgerCountries(militaryAirAt(index, 10, 12), { kind: 'selected' }, [countryKey(3)], undefined,
    12, 14, 'taken', data.countries).rows.map(row => row.identity)), new Set(['3:9']), 'the dashboard’s slot is its holders under any tag, never another holder of one');
  const missing = ledgerCountries(militaryAirAt(index, 10, 12), { kind: 'tags', tags: ['unknown'] }, [], undefined, 12, 14, 'taken', data.countries);
  assert.deepEqual(missing.rows, []); assert.equal(missing.reason, 'Selected countries are unavailable in this campaign.');
  assert.deepEqual(ledgerCountries(militaryAirAt(index, 10, 12), { kind: 'tags', tags: ['X'] }, [], undefined, 12, 14).rows, [],
    'missing native metadata never falls back to literal tags');
  assert.equal(ledgerCountries(militaryAirAt(index, 10, 10), { kind: 'all' }, [], undefined, 10, 14, 'taken', data.countries).rows.length, 3);
});

test('line hover uses exact wide signed values, native/read dates, dated labels and unknown coverage', () => {
  const wide = 9007199254740993n * 100000n + 12345n;
  const data = reply({ leaves: [leaf({ points: [[9, 10, wide.toString()]] }),
    leaf({ cause: 3, points: [[8, 11, '-100000']] }),
    leaf({ dealt: true, opponent: 'other', points: [[9, 10, (-wide).toString()]] })] });
  const index = militaryAirIndex(validateMilitaryAirReply(data));
  const labelled: number[] = [];
  const labelAt = (hour: number) => { labelled.push(hour); return () => ({ name: `Dated name ${hour}`, flag: `dated-flag-${hour}` }); };
  const tip = militaryAirLineTooltip(index, 10, 11, ['mod country'], labelAt, () => '#123456');
  const value = (label: string) => tip.rows.find(row => row.label === label)?.value;
  assert.equal(tip.rows[0].label, 'Dated name 11');
  assert.equal(tip.rows[0].icon, 'dated-flag-11');
  assert.deepEqual(tip.rows[0].countryFlag, { name: 'Dated name 11', flag: 'dated-flag-11' });
  assert.equal(tip.rows[0].swatch, '#123456');
  assert.equal(value('Own losses'), '9,007,199,254,740,992.12345');
  assert.equal(value('Enemy losses'), '−9,007,199,254,740,993.12345');
  assert.equal(value('Operational losses (accidents)'), '−1');
  assert.equal(value('Other losses with unknown opponent'), '9,007,199,254,740,993.12345');
  assert.equal(value('Ratio'), '-1.00 : 1');
  assert.equal(value('Last supplied native event'), undefined);
  assert.equal(value('Event supplied'), undefined);
  assert.match(value('Aircraft combat')!, /9,007,199,254,740,993\.12345 lost/);
  assert.deepEqual(labelled, [11]);
  const gap = militaryAirLineTooltip(index, 10, 12, [], labelAt, () => '#123456');
  assert.equal(gap.rows[0].label, 'Not recorded');
  assert.equal(gap.rows[0].countryFlag, undefined);
  const incomplete = militaryAirLineTooltip(index, 10, 13, ['mod country'], labelAt, () => '#123456');
  assert.equal(incomplete.rows.find(row => row.label === 'Ratio')?.value, 'Not available');
  assert.equal(incomplete.rows.find(row => row.label === 'Event supplied'), undefined);
  assert.match(incomplete.note!, /Some losses may be missing/);
  for (const hour of [11, 13]) {
    const native = { name: `Native holder at ${hour}`, flagPending: hour === 13,
      flagReason: hour === 11 ? 'The shown read refused this flag.' : undefined };
    const dated = militaryAirLineTooltip(index, 10, hour, ['mod country'], at => {
      assert.equal(at, hour); return () => native;
    }, () => '#123456');
    assert.equal(dated.rows[0].countryFlag, native, 'the native heading forwards the complete hovered-read payload');
    assert.equal(dated.rows[0].icon, undefined, 'missing artwork does not erase native flag status');
  }
});

test('the bombing breakdown writes building levels to two decimals, and a sliver of damage never as none', () => {
  assert.equal(ledgerLabel(772204602n, 'bombing'), '77.22');
  assert.equal(ledgerLabel(1083804602n, 'bombing'), '108.38');
  assert.equal(ledgerLabel(1000n, 'bombing'), '0.0001');
});
