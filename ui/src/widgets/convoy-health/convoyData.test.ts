import test from 'node:test';
import assert from 'node:assert/strict';
import { convoyRows, dayPeriods, factsOrReason, heatRows, labelDates, validateConvoys } from './convoyData.ts';

const all = [{ firstHour: 0, lastHour: 95 }];
const base = { branchId: '1', tag: 'IMP', firstHour: 0, lastHour: 95, scale: 100000, warnings: [], known: [all, all, all], unheld: [] };
const names = (tag: string | null) => ({ name: tag ?? '?' });

test('days read their last recorded hour and a day the recording missed is a gap', () => {
  const periods = dayPeriods([0, 95], [{ firstHour: 20, lastHour: 47 }]);
  assert.deepEqual(periods.map(period => period.readHour), [19, null, 71, 95]);
});

test('regions draw their share, blank where not read, never 100%, and rank over the whole span', () => {
  const reply = validateConvoys('regions')({ ...base, view: 'regions', deals: [], leases: [], resources: [],
    regions: [{ region: 7, name: 'Synthetic Sea', missing: null, efficiency: [[0, 50, 40000]] },
      { region: 8, name: null, missing: 'Region 8 absent from the recorded stack', efficiency: [[90, 95, 100000]] }] });
  const rows = heatRows(convoyRows(reply, names), dayPeriods([0, 95], []), '#000', 10, reply.scale);
  assert.deepEqual(rows.map(row => row.label), ['Synthetic Sea', 'Sea region 8']);
  assert.deepEqual(rows[0].cells.map(cell => cell.parts[0]?.share ?? null), [0.4, 0.4, null, null]);
  assert.deepEqual(heatRows(convoyRows(reply, names), dayPeriods([0, 95], []), '#000', 1, reply.scale).map(row => row.label), ['Synthetic Sea']);
  assert.equal(convoyRows(reply, names)[1].nameDetail, 'Region 8 absent from the recorded stack');
});

test('each deal is its own row and lend-lease is hatched as not recorded', () => {
  const reply = validateConvoys('trade')({ ...base, view: 'trade', regions: [],
    resources: [{ id: 0, token: 'synthetic_ore', name: 'Synthetic ore' }],
    deals: [{ id: '0000000000000001', exporter: [{ firstHour: 0, lastHour: 95, tag: 'EXP' }], resource: [[0, 95, 0]], efficiency: [[0, 95, 50000]],
      factors: [[[0, 95, 99999]], [[0, 95, 50001]]], convoys: [[[0, 95, 3]], [[0, 95, 5]]] }],
    leases: [{ handle: '9', giver: [{ firstHour: 24, lastHour: 95, tag: 'EXP' }], bad: [[24, 95, 1]], convoys: [[[24, 95, 1]], [[24, 95, 4]]] }] });
  const rows = convoyRows(reply, names);
  assert.deepEqual(rows.map(row => row.label), ['EXP · Synthetic ore', 'Lend-lease from EXP']);
  const heat = heatRows(rows, dayPeriods([0, 95], []), '#000', 10, reply.scale);
  assert.equal(heat[0].cells[0].parts[0].share, 0.5);
  assert.deepEqual(heat[1].cells.map(cell => cell.unknown === true), [false, true, true, true]);
  assert.equal(rows[0].facts(10)[2].value, '99.99% × 50.00%');
  assert.equal(rows[0].facts(10)[3].value, '3 of 5');
  assert.deepEqual(rows[1].facts(10), []);
  assert.equal(rows[1].facts(30)[0].value, 'Not recorded');
  assert.equal(rows[1].facts(30)[1].value, 'Drawn as a bad route');
});

test('a lend-lease client whose giver has no tag keeps its route and convoys', () => {
  const reply = validateConvoys('trade')({ ...base, view: 'trade', regions: [], resources: [], deals: [],
    leases: [{ handle: '9', giver: [{ firstHour: 0, lastHour: 95, tag: null }], bad: [[0, 95, 0]], convoys: [[[0, 95, 1]], [[0, 95, 4]]] }] });
  const [row] = convoyRows(reply, names);
  assert.equal(row.label, 'Lend-lease from ?');
  assert.equal(row.unknownAt(10), true);
  assert.equal(row.facts(10).at(-1)!.value, '1 of 4');
});

test('a cell where the table was not read is unknown, never blank, and an unheld day says why', () => {
  const reply = validateConvoys('regions')({ ...base, view: 'regions', deals: [], leases: [], resources: [], unheld: [{ firstHour: 72, lastHour: 95 }],
    known: [[{ firstHour: 0, lastHour: 47 }], [], []], regions: [{ region: 7, name: 'Synthetic Sea', missing: null, efficiency: [[0, 23, 40000]] }] });
  const rows = convoyRows(reply, names);
  const periods = dayPeriods([0, 95], []);
  const [row] = heatRows(rows, periods, '#000', 10, reply.scale);
  assert.deepEqual(row.cells.map(cell => cell.unknown === true ? 'unknown' : cell.parts.length ? 'value' : 'blank'), ['value', 'blank', 'unknown', 'unknown']);
  assert.equal(factsOrReason(rows[0], periods[1], reply).reason, 'No bad convoy route here');
  assert.equal(factsOrReason(rows[0], periods[2], reply).reason, 'Not recorded');
  assert.equal(factsOrReason(rows[0], periods[3], reply).reason, 'The country was not held then');
  assert.equal(factsOrReason(rows[0], periods[0], reply).facts[0].value, '40.00%');
});

test('labels ask names at the end of each run that names a country', () => {
  const reply = validateConvoys('trade')({ ...base, view: 'trade', regions: [], resources: [],
    deals: [{ id: '1', exporter: [{ firstHour: 0, lastHour: 40, tag: 'OLD' }, { firstHour: 41, lastHour: 95, tag: 'NEW' }], resource: [], efficiency: [[0, 95, 1]], factors: [[], []], convoys: [[], []] }],
    leases: [{ handle: '9', giver: [{ firstHour: 0, lastHour: 10, tag: null }], bad: [[0, 10, 0]], convoys: [[], []] }] });
  assert.deepEqual(labelDates(reply), [{ hour: 40, tag: 'OLD' }, { hour: 95, tag: 'NEW' }]);
  const asked: [string | null, number][] = [];
  const [deal] = convoyRows(reply, (tag, hour) => { asked.push([tag, hour]); return { name: tag ?? '?' }; });
  assert.equal(deal.label, 'NEW · Resource name unavailable');
  asked.length = 0;
  assert.equal(deal.facts(20)[0].value, 'OLD');
  assert.deepEqual(asked, [['OLD', 40]]);
});

test('a reply of another view or with a malformed run is refused', () => {
  assert.throws(() => validateConvoys('trade')({ ...base, view: 'regions', regions: [], deals: [], leases: [], resources: [] }));
  assert.throws(() => validateConvoys('regions')({ ...base, view: 'regions', deals: [], leases: [], resources: [],
    regions: [{ region: 7, name: null, missing: null, efficiency: [[5, 4, 1]] }] }));
});
