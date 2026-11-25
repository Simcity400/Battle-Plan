import assert from 'node:assert/strict';
import test from 'node:test';
import { BOMBING_SUNBURST_TEXT, partAmount, ledgerComplete, ledgerCountryAt, ledgerHeaderTag, ledgerPinnedIdentity, ledgerTotals, militaryAirAt, militaryAirIndex } from './militaryAirLedger.ts';
import type { MilitaryAirPart } from '../generated/dashboard';
import type { MilitaryAirLedger } from './militaryAirLedger';

const span = (firstHour: number, lastHour = firstHour) => ({ firstHour, lastHour });
type Point = [number, number, string];
interface Leaf { tag: string | null; identity: string | null; dealt: boolean; opponent: string | null; cause: number; points: Point[] }
const leaf = (extra: Partial<Leaf> = {}): Leaf => ({ tag: 'mod country', identity: '3:9', dealt: false, opponent: null, cause: 1,
  points: [[9, 10, '300000'], [8, 11, '200000']], ...extra });
const part = (item: Leaf): MilitaryAirPart => ({ tag: item.tag, identity: item.identity, dealt: item.dealt, cause: item.cause,
  opponentKnown: item.opponent !== null, buildingToken: null, buildingId: null, state: null, points: item.points });
const reply = ({ leaves = [leaf()], ...extra }: Partial<MilitaryAirLedger> & { leaves?: Leaf[] } = {}): MilitaryAirLedger => ({ family: 'airLosses', firstHour: 10, lastHour: 14, causes: [{ cause: 1, name: 'Aircraft combat', missing: null }, { cause: 3, name: 'Air accidents', missing: null }], 
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

test('partAmount reads delta amounts over inclusive spans', () => {
  const data = reply();
  assert.equal(partAmount(data.parts[0], 10, 10), 300000n);
  assert.equal(partAmount(data.parts[0], 10, 11), 200000n);
  const rangeLeaf = part(leaf({ points: [[11, 12, '-100000']] }));
  assert.equal(partAmount(rangeLeaf, 11, 11), 0n);
  assert.equal(partAmount(rangeLeaf, 11, 12), -100000n);
});

test('empty complete reads certify zero, while gaps and unknown bombing callers do not', () => {
  const index = militaryAirIndex(reply({ leaves: [] }));
  assert.equal(militaryAirAt(index, 10, 10)[0].taken, 0n);
  assert.equal(militaryAirAt(index, 10, 10)[0].complete, true);
  assert.deepEqual(militaryAirAt(index, 10, 12), []);
  assert.equal(militaryAirAt(index, 10, 13)[0].complete, false);
  assert.equal(ledgerComplete([span(10, 14)], 10, 13, [span(11)]), false);
});

test('shared indices retain wide signed corrections exactly', () => {
  const wide = (1n << 80n).toString();
  const data = reply({ leaves: [leaf({ points: [[10, 10, wide]] }),
    leaf({ cause: 3, points: [[10, 10, '-100000']] }),
    leaf({ dealt: true, opponent: 'other mod', points: [[10, 10, '700000']] })] });
  const index = militaryAirIndex(data);
  assert.equal(militaryAirIndex(data), index);
  const row = militaryAirAt(index, 10, 10)[0];
  assert.equal(row.taken, BigInt(wide) - 100000n);
  assert.equal(row.dealt, 700000n);
});

test('country aliases share their holder total', () => {
  const data = reply({ observed: [span(10, 14)], countries: [
    { tag: 'mod country', identity: '3:9', present: [span(10)], membership: [span(10)] },
    { tag: 'renamed', identity: '3:9', present: [span(11, 14)], membership: [span(13)] },
    { tag: 'mod country', identity: '8:99', present: [span(13, 14)], membership: [] },
  ], leaves: [leaf({ points: [[9, 10, '300000']] })] });
  const index = militaryAirIndex(data);
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
  assert.equal(ledgerCountryAt(index, 'mod country', 10, 9), null);
  assert.equal(ledgerHeaderTag(index, null, 'mod country', 9, undefined, null), 'mod country');
  assert.equal(ledgerHeaderTag(index, null, 'mod country', 9, '8:99', { tag: 'mod country', identity: '8:99' }), null, 'a pinned holder without a row is not named by its tag');
  assert.equal(ledgerHeaderTag(index, ledgerCountryAt(index, 'mod country', 10, 10), 'mod country', 10, undefined, null), 'mod country');
  assert.equal(ledgerHeaderTag(index, ledgerCountryAt(index, 'mod country', 10, 13), 'mod country', 13, undefined, null), 'renamed', 'the holder shown, under its tag at the hour');
  assert.equal(ledgerHeaderTag(index, ledgerCountryAt(index, 'mod country', 10, 13, pin), 'mod country', 13, pin, { tag: 'mod country', identity: '8:99' }), 'mod country');
});

test('the bombing breakdown writes building levels to two decimals, and a sliver of damage never as none', () => {
  assert.equal(BOMBING_SUNBURST_TEXT.format(77.2204602, 772204602n), '77.22');
  assert.equal(BOMBING_SUNBURST_TEXT.format(108.3804602, 1083804602n), '108.38');
  assert.equal(BOMBING_SUNBURST_TEXT.format(0.0001, 1000n), '0.0001');
  assert.equal(BOMBING_SUNBURST_TEXT.format(0, null), 'Not recorded');
});
