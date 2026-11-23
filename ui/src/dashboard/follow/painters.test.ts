import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, parseFollowed } from './message.ts';
import { message } from './messages.fixture.ts';
import { BarCell, TextCell, formatAmountFast, paintArmy, paintMeasure, paintStability } from './painters.ts';
import { formatAmount } from '../../charts/valueAxis.ts';


function cell() {
  const node = { data: '' }, host = { title: '' };
  return { cell: new TextCell(node, host), node, host };
}

test('the measure and stability painters write values, and a missing value as its reason, never zero', () => {
  const slots = () => [cell(), cell(), cell()];
  const answer = parseFollowed(message(10, [{ id: 1, answer: {
    keys: [1n, 2n, 0n], columns: 1, values: [123_456_789n, 0n, 0n],
    states: [CELL.value, CELL.unrecorded, CELL.noRow], reasons: [null, "SOV's politicalPower was not recorded at this hour", 'ITA is held by no country'],
  } }])).answers[0]!.answer;
  const measure = slots();
  paintMeasure(answer, measure.map(slot => slot.cell), 5, 1);
  assert.deepEqual(measure.map(slot => [slot.node.data, slot.host.title]),
    [['1,234.6', ''], ['—', "SOV's politicalPower was not recorded at this hour"], ['—', 'ITA is held by no country']]);
  const stability = slots();
  paintStability(parseFollowed(message(10, [{ id: 1, answer: { keys: [1n], columns: 1, values: [61_500n] } }])).answers[0]!.answer, [stability[0]!.cell], 5, 1);
  assert.equal(stability[0]!.node.data, '61.5%');
  paintStability({ kind: 'notRecorded', reason: 'CountryStability was not recorded at this hour' }, stability.map(slot => slot.cell), 5, 0);
  assert.deepEqual(stability.map(slot => slot.node.data), ['—', '—', '—']);
  assert.equal(stability[2]!.host.title, 'CountryStability was not recorded at this hour');
  let writes = 0;
  const counted = { get data() { return ''; }, set data(_: string) { writes++; } };
  const unchanged = [new TextCell(counted, { title: '' })];
  paintMeasure(answer, unchanged, 5, 1);
  paintMeasure(answer, unchanged, 5, 1);
  assert.equal(writes, 1, 'an unchanged text is not written again');
});

test('the army painter draws each row as a share of the greatest', () => {
  const rows = Array.from({ length: 3 }, () => { const style = { width: '' }; const value = cell(); return { style, value, row: { bar: new BarCell(style), value: value.cell } }; });
  const answer = parseFollowed(message(10, [{ id: 1, answer: { keys: [4n, 9n, 2n], columns: 1, values: [200n, 50n, 0n], states: [CELL.value, CELL.value, CELL.unrecorded], reasons: [null, null, 'Not recorded'] } }])).answers[0]!.answer;
  paintArmy(answer, rows.map(row => row.row), 0, 0);
  assert.deepEqual(rows.map(row => [row.style.width, row.value.node.data]), [['100%', '200'], ['25%', '50'], ['0%', '—']]);
  assert.equal(rows[2]!.value.host.title, 'Not recorded');
  paintArmy({ kind: 'notRecorded', reason: 'ArmyTotals was not recorded at this hour' }, rows.map(row => row.row), 0, 0);
  assert.deepEqual(rows.map(row => [row.style.width, row.value.node.data]), [['0%', '—'], ['0%', '—'], ['0%', '—']]);
});

test('the fast amount writes every value as formatAmount does', () => {
  const edges = [0n, 1n, 4n, 5n, 49_999n, 50_000n, 99_999n, 100_000n, 150_000n, 123_456_789n, 9_007_199_254_740_991n, 9_007_199_254_740_992n, 1n << 62n];
  let seed = 12345n;
  const raws = [...edges, ...edges.map(raw => -raw)];
  for (let i = 0; i < 2000; i++) {
    seed = (seed * 6364136223846793005n + 1442695040888963407n) & ((1n << 64n) - 1n);
    raws.push(BigInt.asIntN(64, seed) >> BigInt(i % 60));
  }
  for (const raw of raws) for (const scale of [0, 2, 5]) for (let digits = 0; digits <= 6; digits++) {
    assert.equal(formatAmountFast(raw, scale, digits), formatAmount(raw, scale, digits), `${raw} at scale ${scale}, ${digits} digits`);
  }
});

