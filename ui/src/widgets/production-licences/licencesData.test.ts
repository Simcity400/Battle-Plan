import test from 'node:test';
import assert from 'node:assert/strict';
import { holderLicencesAt, holdersAt, licencePairs, licencesAt, rosterKnown, validateLicences } from './licencesData.ts';

const reply = () => validateLicences({ branchId: '1', view: 'licences', lastHour: 50, known: [[0, 50, '1']], organisations: [], designs: [], designsFrom: null, texts: [], icons: {}, warnings: [],
  holders: { '1': [{ hour: 0, tag: 'LIC' }], '2': [{ hour: 0, tag: 'GIV' }], '3': [{ hour: 0, tag: 'OTH' }] },
  variants: { c1: [{ firstHour: 0, name: 'Synthetic fighter', known: true, design: null }] },
  licences: [
    { licensee: '1', ordinal: 0, licensor: [[10, 30, '2']], lent: [[10, 30, '0']], required: [[10, 30, '1']], start: [[10, 30, '900']], variants: [[[10, 30, 'c1']], [[10, 30, 'c2']]] },
    { licensee: '1', ordinal: 1, licensor: [[10, 30, '2']], lent: [[10, 30, '3']], required: [], start: [], variants: [] },
    { licensee: '3', ordinal: 0, licensor: [[20, 30, '3']], lent: [], required: [], start: [], variants: [] },
  ],
  coverage: { firstHour: 0, lastHour: 50, complete: true }, pending: [] });

test('licences keep every design, known zero factories and their parties at the hour', () => {
  const rows = licencesAt(reply(), 15, () => true);
  assert.equal(rows.length, 2);
  assert.deepEqual(rows[0].designs, [{ variant: 'c1', name: 'Synthetic fighter' }, { variant: 'c2', name: null }]);
  assert.equal(rows[0].lent, 0n);
  assert.equal(rows[1].since, null);
  assert.equal(licencesAt(reply(), 40, () => true).length, 0);
  assert.equal(licencesAt(reply(), 25, tag => tag === 'OTH').length, 1);
});

test('ribbons count licences, or the factories lent where the measure asks', () => {
  const rows = licencesAt(reply(), 25, () => true);
  assert.deepEqual(licencePairs(rows, 'licences').map(pair => [pair.licensor, pair.licensee, pair.width]), [['GIV', 'LIC', 2], ['OTH', 'OTH', 1]]);
  // A licence lending no factories, or none recorded, adds no width.
  assert.deepEqual(licencePairs(rows, 'factories').map(pair => pair.width), [3, 0]);
});

test('a picked country keeps its holders through a rename, never the next holder of its tag', () => {
  const renamed = reply();
  renamed.holders['2'] = [{ hour: 0, tag: 'GIV' }, { hour: 20, tag: 'NEW' }];
  renamed.holders['3'] = [{ hour: 0, tag: 'OTH' }, { hour: 22, tag: 'GIV' }];
  const holders = holdersAt(renamed, 'GIV', 15);
  assert.deepEqual(holders, ['2']);
  assert.deepEqual(holderLicencesAt(renamed, holders, 25).map(row => [row.licensor, row.licensee]), [['NEW', 'LIC'], ['NEW', 'LIC']]);
});

test('a refused licence roster is unknown, not zero licences', () => {
  assert.equal(rosterKnown(reply(), 15), true);
  assert.equal(rosterKnown({ known: [[0, 9, '1']] }, 15), false);
});
