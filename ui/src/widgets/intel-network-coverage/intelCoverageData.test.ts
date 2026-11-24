import { test } from 'node:test';
import assert from 'node:assert/strict';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { cellState, countriesAt, coverageStructure, matrixCells, pairOf, pairOptions, pairsAt, pairTooltip, validateIntelCoverage } from './intelCoverageData.ts';
import type { IntelCoverageReply } from '../../generated/dashboard';

const ids: Record<string, number> = { GER: 1, ENG: 2, SOV: 3, FRA: 4, ITA: 5 };
const network = (owner: string, target: string) => ({ owner: { id: ids[owner], tag: owner }, target: { id: ids[target], tag: target } });
const reply = (networks: [string, string][], missing: string | null = null): IntelCoverageReply => ({
  countries: Object.keys(ids).sort().map(tag => ({ id: ids[tag], tag })), networks: networks.map(([owner, target]) => network(owner, target)), readHour: 100, missing, warnings: [],
});
const cells = (entries: Record<string, number>) => valuesOf(new Map(Object.entries(entries)), 'Not read yet');

test('directions are independent; a recorded zero is an empty network; no network is proven only where the hour is known', () => {
  const data = reply([['GER', 'ENG']]);
  const at = pairsAt(data);
  const values = cells({ [liveKey('1:2', 'coveredControlled')]: 0 });
  const present = cellState(at, values, 'GER', 'ENG');
  assert.equal(present.kind, 'present');
  assert.equal(present.kind === 'present' && present.values.get('coveredControlled'), 0);
  assert.equal(present.kind === 'present' && present.values.get('maxStrength'), 'Not read yet');
  assert.equal(cellState(at, values, 'ENG', 'GER').kind, 'absent');
  assert.equal(cellState(pairsAt(reply([['GER', 'ENG']], 'IntelNetworkSummaries: not recorded')), values, 'ENG', 'GER').kind, 'unknown');
  assert.equal(cellState(at, values, 'ENG', 'XXX').kind, 'unknown');
});

test('the most active countries are those with the most networks in or from them, ties by tag', () => {
  const data = reply([['GER', 'ENG'], ['GER', 'SOV'], ['SOV', 'GER'], ['ITA', 'FRA']]);
  assert.deepEqual(countriesAt(data, 3), ['GER', 'SOV', 'ENG']);
  assert.deepEqual(countriesAt(data, 10), ['GER', 'SOV', 'ENG', 'FRA', 'ITA']);
  assert.deepEqual(countriesAt(data, 3, tag => tag === 'SOV' || tag === 'FRA'), ['FRA', 'SOV']);
  assert.deepEqual(countriesAt(data, 3, tag => tag === 'SOV', true), ['SOV', 'GER']);
  assert.deepEqual(countriesAt(data, 1, undefined, true), ['GER', 'ENG', 'SOV']);
  assert.deepEqual(countriesAt(data, 3, () => false, true), []);
});


test('the matrix colours each network against the largest shown, leaves the diagonal and proven absences empty, hatches the unknown', () => {
  const at = pairsAt(reply([['GER', 'ENG'], ['SOV', 'GER']]));
  const seen: [number, number][] = [];
  const matrix = matrixCells(at, cells({ [liveKey('1:2', 'coveredControlled')]: 4, [liveKey('3:1', 'coveredControlled')]: 8 }), 'coveredControlled',
    (value, highest) => { seen.push([value, highest]); return `c${value}`; });
  assert.deepEqual(matrix.get(liveKey('GER', 'ENG')), { color: 'c4' });
  assert.deepEqual(seen, [[4, 8]]);
  assert.equal(matrix.get(liveKey('GER', 'GER')), null);
  assert.equal(matrix.get(liveKey('ENG', 'GER')), null);
  assert.equal(matrixCells(at, cells({}), 'coveredControlled', () => 'x').get(liveKey('GER', 'ENG')), undefined);
  assert.equal(matrix.get(liveKey('GER', 'XXX')), undefined);
});

test('tooltips keep unknown, no network and recorded values apart, with the read date', () => {
  const at = pairsAt(reply([['GER', 'ENG']]));
  const values = cells({ [liveKey('1:2', 'coveredControlled')]: 3 });
  const present = pairTooltip('Germany', 'England', cellState(at, values, 'GER', 'ENG'), '1 Jan 1937');
  assert.equal(present.title, 'Germany → England');
  assert.equal(present.rows.find(row => row.label === 'Controlled states covered')?.value, '3');
  assert.equal(present.rows.find(row => row.label === 'Strongest state')?.value, 'Not recorded (Not read yet)');
  assert.equal(present.rows.at(-1)?.value, '1 Jan 1937');
  assert.deepEqual(pairTooltip('A', 'B', { kind: 'absent' }, null).rows, [{ label: 'Network', value: 'No network' }]);
  assert.deepEqual(pairTooltip('A', 'B', { kind: 'unknown' }, null).rows, [{ label: 'Network', value: 'Not recorded' }]);
});

test('the daily read hour and the values are not structure; which pairs hold a network is', () => {
  const data = reply([['GER', 'ENG']]);
  assert.equal(coverageStructure(data), coverageStructure({ ...data, readHour: 124 }));
  assert.notEqual(coverageStructure(data), coverageStructure(reply([['GER', 'ENG'], ['ENG', 'GER']])));
  assert.equal(validateIntelCoverage(data), data);
  assert.throws(() => validateIntelCoverage({ ...data, countries: [{ id: 1, tag: null }] }));
});

test('a pinned pair keeps its owner, the hour it was chosen at, and its target with the raw id its line is read by', () => {
  const pair = { owner: 'GER', target: 'ENG', targetId: 2, hour: 512 };
  const saved = pairOptions(pair);
  assert.deepEqual(pairOf(saved.owner, saved.target), pair);
  assert.equal(pairOf('', saved.target), null);
  assert.equal(pairOf('GER|x', saved.target), null);
});
