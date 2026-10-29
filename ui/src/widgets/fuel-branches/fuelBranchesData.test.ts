import test from 'node:test';
import assert from 'node:assert/strict';
import type { FuelBranchesReply } from '../../generated/dashboard';
import { branchesAt, chartCountryAt, consumersKnown, countryAt, emptyTimeView, memberAt, memberRatioRuns, ratioAxis, ratioRuns, validateFuelBranches } from './fuelBranchesData.ts';

const reply = (): FuelBranchesReply => ({
  branchId: '1', lastHour: 40, consumerNames: ['Land', null, 'Sea', null], scale: 100000, warnings: [], known: [[10, 40, '1']],
  countries: [
    { holder: '3', tags: [{ hour: 0, tag: 'AAA' }, { hour: 30, tag: 'BBB' }], read: [[10, 33, '10']], branches: [
      { index: 0, requested: [[10, 20, '500000'], [21, 33, '0']], received: [[10, 33, '250000']], priority: [[10, 33, '1']] },
      { index: 3, requested: [[10, 33, '100']], received: [[10, 33, '900']], priority: [] },
    ] },
    // The tag reused by another country later.
    { holder: '4', tags: [{ hour: 30, tag: 'AAA' }], read: [[30, 40, '30']], branches: [
      { index: 0, requested: [[30, 40, '7']], received: [[30, 40, '7']], priority: [] },
    ] },
  ],
  coverage: { firstHour: 0, lastHour: 40, complete: true }, pending: [],
} as unknown as FuelBranchesReply);

test('fuel by branch keeps the recorded branches, unmet only when positive and unknown apart from zero', () => {
  const data = validateFuelBranches(reply());
  const country = countryAt(data, 'AAA', 15)!;
  assert.equal(country.holder, '3');
  const [land, fourth] = branchesAt(data, country, 15);
  assert.deepEqual([land.name, land.requested, land.received, land.unmet, land.priority], ['Land', 500000n, 250000n, 250000n, 1]);
  // A content-unnamed branch from the recording still gets its row; receipt above request leaves nothing unmet.
  assert.deepEqual([fourth.index, fourth.name, fourth.unmet], [3, 'Branch name unavailable', 0n]);
  // Before the first read nothing is known, not zero.
  assert.equal(branchesAt(data, country, 5)[0].requested, null);
  // A reused tag reads the holder that had it then.
  assert.equal(countryAt(data, 'AAA', 35)!.holder, '4');
});

test('received share lines leave hours without a request as gaps', () => {
  const runs = ratioRuns(reply().countries[0].branches[0]);
  assert.deepEqual(runs, [{ firstHour: 10, lastHour: 20, value: 50, breakBefore: true }]);
});

test('a malformed reply is refused', () => {
  const bad = reply() as unknown as { countries: { branches: { requested: unknown }[] }[] };
  bad.countries[0].branches[0].requested = [[5, 4, '1']];
  assert.throws(() => validateFuelBranches(bad));
});

test('a reused tag selects the holder read at the hour, as the Rust fixture records it', () => {
  // The dashboard-economy fixture: holder 3 is AAA at 10, holder 4 is AAA from 11.
  const data = validateFuelBranches({ branchId: '1', lastHour: 11, consumerNames: [], scale: 100000, warnings: [], known: [[10, 11, '1']],
    countries: [
      { holder: '3', tags: [{ hour: 10, tag: 'AAA' }], read: [[10, 10, '10']], branches: [{ index: 3, requested: [[10, 10, '9']], received: [[10, 10, '9']], priority: [] }] },
      { holder: '4', tags: [{ hour: 11, tag: 'AAA' }], read: [[11, 11, '11']], branches: [{ index: 3, requested: [[11, 11, '9']], received: [[11, 11, '9']], priority: [] }] },
    ], coverage: { firstHour: 10, lastHour: 11, complete: true }, pending: [] });
  assert.equal(countryAt(data, 'AAA', 10)!.holder, '3');
  assert.equal(countryAt(data, 'AAA', 11)!.holder, '4');
  assert.equal(chartCountryAt(data, 'AAA', 11)!.holder, '4');
});

test('a never-readable consumer table is not recorded, not no demand', () => {
  const data = validateFuelBranches({ ...reply(), known: [] });
  assert.equal(consumersKnown(data, 15), false);
  assert.equal(consumersKnown(reply(), 15), true);
});

test('an empty time view tells pending, unrecorded and proven-empty hours apart', () => {
  const base = reply();
  // The table was read from hour 10: before it nothing is proven.
  assert.equal(emptyTimeView(base, 15), 'none');
  assert.equal(emptyTimeView(base, 5), 'unrecorded');
  // A cold prefix: nothing prepared yet.
  assert.equal(emptyTimeView({ ...base, lastHour: null, known: [] } as unknown as FuelBranchesReply, 5), 'pending');
  // Hours past the prepared prefix are pending.
  assert.equal(emptyTimeView({ ...base, known: [], pending: [{ firstHour: 41, lastHour: 60 }] } as unknown as FuelBranchesReply, 50), 'pending');
  // Read through 20, then pending 21–50: hour 30 waits, never 'no fuel so far'.
  const pending = validateFuelBranches({ ...base, lastHour: 60, known: [[10, 20, '1']], pending: [{ firstHour: 21, lastHour: 50 }] });
  assert.equal(emptyTimeView(pending, 30), 'pending');
  assert.equal(emptyTimeView(pending, 15), 'none');
  // A later hour the table was not read is unrecorded, whatever came before.
  const gap = validateFuelBranches({ ...base, lastHour: 60, known: [[10, 20, '1'], [31, 60, '1']] });
  assert.equal(emptyTimeView(gap, 25), 'unrecorded');
  assert.equal(emptyTimeView(gap, 40), 'none');
});

test('group membership clips the lines, tooltips and stops to the member hours', () => {
  const within = [{ firstHour: 15, lastHour: 18 }];
  const runs = memberRatioRuns(reply().countries[0].branches[0], within);
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour]), [[15, 18]]);
  assert.equal(memberAt(within, 14), false);
  assert.equal(memberAt(within, 16), true);
  assert.equal(memberAt(null, 1), true);
});

test('the ratio axis keeps 0-100% and grows in round steps to show receipts above the request', () => {
  const line = (...values: number[]) => ({ runs: values.map((value, index) => ({ firstHour: index, lastHour: index, value, breakBefore: index === 0 })) });
  assert.deepEqual(ratioAxis([line(40, 100)]), { top: 100, ticks: [0, 25, 50, 75, 100] });
  assert.deepEqual(ratioAxis([line(40), line(150)]), { top: 150, ticks: [0, 25, 50, 75, 100, 125, 150] });
  assert.deepEqual(ratioAxis([line(900)]), { top: 1000, ticks: [0, 250, 500, 750, 1000] }, 'a 900% receipt is drawn, not clipped at 100%');
  assert.deepEqual(ratioAxis([]), { top: 100, ticks: [0, 25, 50, 75, 100] });
});
