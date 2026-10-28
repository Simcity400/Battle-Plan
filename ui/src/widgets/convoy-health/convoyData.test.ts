import test from 'node:test';
import assert from 'node:assert/strict';
import { cellRuns, heatNavigation, periodAt } from '../../charts/heat/layout.ts';
import { MAX_NAME_HOURS, wantedHours } from '../../dashboard/countryFacts.ts';
import { convoyPending, convoyRows, dayPeriods, heatRows, labelDates, periodPending, shownPeriods, validateConvoys } from './convoyData.ts';

const base = { branchId: '1', lastHour: 95, tag: 'IMP', scale: 100000, warnings: [], known: [[[0, 95, '1']], [[0, 95, '1']], [[0, 95, '1']]], holders: { '1': [{ hour: 0, tag: 'IMP' }], '2': [{ hour: 0, tag: 'EXP' }] },
  coverage: { firstHour: 0, lastHour: 95, complete: true }, pending: [] };
const names = { country: (tag: string | null) => tag ?? '?' };

test('days read their last recorded hour and a day the recording missed is a gap', () => {
  const periods = dayPeriods([0, 95], [{ firstHour: 20, lastHour: 47 }]);
  assert.deepEqual(periods.map(period => period.readHour), [19, null, 71, 95]);
});

test('regions draw their share, blank where not read, never 100%', () => {
  const reply = validateConvoys('regions')({ ...base, view: 'regions', deals: [], leases: [], resources: [],
    regions: [{ region: 7, name: 'Synthetic Sea', efficiency: [[0, 50, '40000']] }, { region: 8, name: null, efficiency: [[90, 95, '100000']] }] });
  const periods = dayPeriods([0, 95], []);
  const rows = heatRows(convoyRows(reply, names), periods, '#000', 10);
  assert.deepEqual(rows.map(row => row.label), ['Synthetic Sea', 'Sea region 8']);
  assert.deepEqual(rows[0].cells.map(cell => cell.parts[0]?.share ?? null), [0.4, 0.4, null, null]);
  // Following at hour 40, only completed days rank: the later region is left out.
  assert.deepEqual(heatRows(convoyRows(reply, names), periods, '#000', 10, 40).map(row => row.label), ['Synthetic Sea']);
});

test('each deal is its own row and lend-lease is hatched as not recorded', () => {
  const reply = validateConvoys('trade')({ ...base, view: 'trade', regions: [],
    resources: [{ id: 0, token: 'synthetic_ore', name: 'Synthetic ore' }],
    deals: [{ id: '0000000000000001', exporter: [[0, 95, '2']], resource: [[0, 95, '0']], efficiency: [[0, 95, '50000']], factors: [[[0, 95, '99999']], [[0, 95, '50001']]], convoys: [[[0, 95, '3']], [[0, 95, '5']]] }],
    leases: [{ handle: '9', giver: [[24, 95, '2']], bad: [[24, 95, '1']], convoys: [[[24, 95, '1']], [[24, 95, '4']]] }] });
  const rows = convoyRows(reply, names);
  assert.deepEqual(rows.map(row => row.label), ['EXP · Synthetic ore', 'Lend-lease from EXP']);
  const heat = heatRows(rows, dayPeriods([0, 95], []), '#000', 10);
  assert.equal(heat[0].cells[0].parts[0].share, 0.5);
  assert.deepEqual(heat[1].cells.map(cell => cell.unknown === true), [false, true, true, true]);
  assert.equal(rows[0].detail(10)[2].value, '99.99% × 50%');
  assert.equal(rows[0].detail(10)[3].value, '3 of 5');
  assert.deepEqual(rows[1].detail(10), []);
  assert.equal(rows[1].detail(30)[0].value, 'Not recorded');
});

test('a lend-lease client whose giver has no identity keeps its route and convoys', () => {
  const reply = validateConvoys('trade')({ ...base, view: 'trade', regions: [], resources: [], deals: [],
    leases: [{ handle: '9', giver: [], bad: [[0, 95, '0']], convoys: [[[0, 95, '1']], [[0, 95, '4']]] }] });
  const [row] = convoyRows(reply, names);
  assert.equal(row.label, 'Lend-lease from ?');
  assert.equal(row.unknownAt(10), true);
  assert.equal(row.detail(10).at(-1)!.value, '1 of 4');
});

test('a region cell where the table was not read is unknown, never blank', () => {
  const reply = validateConvoys('regions')({ ...base, view: 'regions', deals: [], leases: [], resources: [],
    known: [[[0, 47, '1']], [], []], regions: [{ region: 7, name: 'Synthetic Sea', efficiency: [[0, 23, '40000']] }] });
  const [row] = heatRows(convoyRows(reply, names), dayPeriods([0, 95], []), '#000', 10);
  // Day 1 was read (no bad route: blank); days 2 and 3 were not read: unknown.
  assert.deepEqual(row.cells.map(cell => cell.unknown === true ? 'unknown' : cell.parts.length ? 'value' : 'blank'), ['value', 'blank', 'unknown', 'unknown']);
});

test('labels ask names at the dates they show', () => {
  const reply = validateConvoys('trade')({ ...base, view: 'trade', regions: [], resources: [],
    deals: [{ id: '1', exporter: [[0, 40, '2'], [50, 95, '2']], resource: [[0, 95, '0']], efficiency: [[0, 95, '50000']], factors: [[], []], convoys: [[], []] }],
    leases: [{ handle: '9', giver: [[24, 60, '2']], bad: [], convoys: [[], []] }] });
  assert.deepEqual(labelDates(reply), { wanted: [{ hour: 95, tag: 'EXP' }, { hour: 60, tag: 'EXP' }], maxHours: MAX_NAME_HOURS });
  const asked: [string | null, number][] = [];
  convoyRows(reply, { country: (tag, hour) => { asked.push([tag, hour]); return tag ?? '?'; } });
  assert.deepEqual(asked, [['EXP', 95], ['EXP', 60]]);
});

test('every deal asks its name at its own date, however many dates there are', () => {
  const count = MAX_NAME_HOURS + 36;
  const reply = validateConvoys('trade')({ ...base, lastHour: count - 1, coverage: { firstHour: 0, lastHour: count - 1, complete: true }, view: 'trade', regions: [], resources: [], leases: [],
    deals: Array.from({ length: count }, (_, index) => ({ id: String(index + 1), exporter: [[0, index, '2']], resource: [], efficiency: [], factors: [[], []], convoys: [[], []] })) });
  const { wanted, maxHours } = labelDates(reply);
  assert.equal(wanted.length, count);
  assert.equal(maxHours, count);
  assert.deepEqual(wantedHours(wanted, maxHours), Array.from({ length: count }, (_, hour) => hour));
});

test('the first day under way shows its reached read, not no activity', () => {
  const reply = validateConvoys('regions')({ ...base, view: 'regions', deals: [], leases: [], resources: [],
    regions: [{ region: 7, name: 'Synthetic Sea', efficiency: [[0, 95, '40000']] }] });
  const periods = shownPeriods(dayPeriods([0, 95], []), [], 10, []);
  const rows = heatRows(convoyRows(reply, names), periods, '#000', 10, 10);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].cells[0].parts[0].share, 0.4);
  // The chart draws the day under way: its read is the hour reached, not the day's end.
  assert.equal(periods[0].readHour, 10);
  const drawn = cellRuns(rows[0], periods, 10);
  assert.equal(drawn.length, 1);
  assert.equal(drawn[0].period.readHour, 10);
  // A pick or tooltip on that day carries the reached read, and keys step to it.
  assert.equal(periods[periodAt(periods, 5)!].readHour, 10);
  assert.ok(heatNavigation(periods).keyHours.includes(10));
  // Only that day changes; days not reached stay as they were and draw nothing yet.
  assert.deepEqual(periods.slice(1), dayPeriods([0, 95], []).slice(1));
});

test('the day under way reads the latest recorded hour reached, skipping a gap', () => {
  const days = dayPeriods([0, 95], [{ firstHour: 30, lastHour: 33 }]);
  const shown = shownPeriods(days, [{ firstHour: 30, lastHour: 33 }], 32, []);
  assert.equal(shown[1].readHour, 29);
  // Reached nothing recorded yet that day: blank, never a future read.
  assert.equal(shownPeriods(dayPeriods([0, 95], [{ firstHour: 24, lastHour: 33 }]), [{ firstHour: 24, lastHour: 33 }], 30, [])[1].readHour, null);
  // A finished day and no playhead keep the same periods.
  assert.equal(shownPeriods(days, [], 47, []), days);
  assert.equal(shownPeriods(days, [], null, []), days);
});

test('a day prepared partway reads its prepared part, and pending days draw as pending, not unrecorded', () => {
  // Prepared through 20, pending 21–95, playhead at 30.
  const reply = validateConvoys('regions')({ ...base, view: 'regions', deals: [], leases: [], resources: [], lastHour: 20,
    known: [[[0, 20, '1']], [], []], pending: [{ firstHour: 21, lastHour: 95 }],
    regions: [{ region: 7, name: 'Synthetic Sea', efficiency: [[0, 20, '40000']] }] });
  const pending = convoyPending(reply, [{ firstHour: 21, lastHour: 95 }], [0, 95]);
  const periods = shownPeriods(dayPeriods([0, 95], []), [], 30, pending);
  // The first day reads its last prepared hour, not hour 23.
  assert.equal(periods[0].readHour, 20);
  // The day under way and later days have nothing prepared: no read, and pending.
  assert.deepEqual(periods.slice(1).map(period => period.readHour), [null, null, null]);
  assert.ok(periods.slice(1).every(period => periodPending(period, pending)));
  assert.equal(periodPending(periods[0], [{ firstHour: 40, lastHour: 95 }]), false);
  const [row] = heatRows(convoyRows(reply, names), periods, '#000', 10, 30);
  // The known part draws its value; pending days draw no cell, never the unknown hatch.
  assert.equal(row.cells[0].parts[0].share, 0.4);
  assert.ok(row.cells.slice(1).every(cell => !cell.parts.length && !cell.unknown));
  const drawn = cellRuns(row, periods, 30);
  assert.deepEqual(drawn.map(({ period }) => [period.firstHour, period.readHour]), [[0, 20]]);
  // Keys stop at prepared reads; a pick on the first day carries hour 20.
  assert.deepEqual(heatNavigation(periods).keyHours, [20]);
  assert.equal(periods[periodAt(periods, 10)!].readHour, 20);
});

test('the pending tail after the last prepared hour joins the reply\'s pending spans', () => {
  assert.deepEqual(convoyPending({ lastHour: 40 }, [{ firstHour: 10, lastHour: 12 }], [0, 95]),
    [{ firstHour: 10, lastHour: 12 }, { firstHour: 41, lastHour: 95 }]);
  assert.deepEqual(convoyPending({ lastHour: null }, [], [0, 95]), [{ firstHour: 0, lastHour: 95 }]);
  assert.deepEqual(convoyPending({ lastHour: 95 }, [], [0, 95]), []);
});
