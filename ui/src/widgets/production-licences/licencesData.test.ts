import test from 'node:test';
import assert from 'node:assert/strict';
import { holderLicencesAt, holdersAt, licencePairs, licencesAt, rosterKnown, validateLicences } from './licencesData.ts';

const reply = () => validateLicences({ branchId: '1', view: 'licences', lastHour: 50, known: [[0, 50, '1']], organisations: [], designs: [], designsFrom: null, texts: [], icons: {}, warnings: [],
  holders: { '1': [{ hour: 0, tag: 'LIC' }], '2': [{ hour: 0, tag: 'GIV' }], '3': [{ hour: 0, tag: 'OTH' }] },
  variants: { c1: [{ firstHour: 0, name: 'Synthetic fighter', known: true, design: null }] },
  licences: [
    { licensee: '1', ordinal: 0, licensor: [[10, 30, '2']], lent: [[10, 30, '0']], required: [[10, 30, '1']], start: [[10, 30, '900']], variants: [[[10, 30, 'c1']], [[10, 30, 'c2']]] },
    { licensee: '1', ordinal: 1, licensor: [[10, 30, '2']], lent: [[10, 30, '3']], required: [], start: [[10, 30, '880']], variants: [] },
    { licensee: '3', ordinal: 0, licensor: [[20, 30, '3']], lent: [], required: [], start: [[20, 30, '890']], variants: [] },
  ],
  coverage: { firstHour: 0, lastHour: 50, complete: true }, pending: [] });

test('licences keep every design, known zero factories and their parties at the hour', () => {
  const { rows } = licencesAt(reply(), 15, () => true);
  assert.equal(rows.length, 2);
  assert.deepEqual(rows[0].designs, [{ variant: 'c1', name: 'Synthetic fighter' }, { variant: 'c2', name: null }]);
  assert.equal(rows[0].lent, 0n);
  // An unrecorded factory count is unknown, not zero.
  assert.equal(rows[1].required, null);
  assert.equal(rows[1].since, 880);
  assert.deepEqual(licencesAt(reply(), 40, () => true), { rows: [], unattributed: 0 });
  assert.equal(licencesAt(reply(), 25, tag => tag === 'OTH').rows.length, 1);
});

test('a licence whose licensor has no identity stands, and may be any country its licensee is not', () => {
  const unnamed = reply();
  // The licensor is identified until 19; the licence stands, its start date read, through 30.
  unnamed.licences[0].licensor = [[10, 19, '2']];
  const all = licencesAt(unnamed, 25, () => true);
  assert.deepEqual(all.rows.map(row => [row.licensor, row.licensorHolder, row.licensee]), [[null, null, 'LIC'], ['GIV', '2', 'LIC'], ['OTH', '3', 'OTH']]);
  assert.equal(all.unattributed, 0);
  assert.deepEqual(licencePairs(all.rows, 'licences').map(pair => [pair.licensor, pair.licensee, pair.width]), [[null, 'LIC', 1], ['GIV', 'LIC', 1], ['OTH', 'OTH', 1]]);
  // GIV is not known to grant it, but may: not drawn, and no proof GIV grants none but the other.
  const giver = licencesAt(unnamed, 25, tag => tag === 'GIV');
  assert.deepEqual([giver.rows.length, giver.unattributed], [1, 1]);
  assert.deepEqual(licencesAt(unnamed, 15, tag => tag === 'GIV').unattributed, 0);
  // Its licensee's own drill still lists it; a possible licensor's counts it, never proving it absent.
  assert.deepEqual(holderLicencesAt(unnamed, ['1'], 25).rows.map(row => row.licensor), [null, 'GIV']);
  const possible = holderLicencesAt(unnamed, ['3'], 25);
  assert.deepEqual([possible.rows.map(row => row.licensee), possible.unattributed], [['OTH'], 1]);
  assert.deepEqual(holderLicencesAt(unnamed, ['2'], 15).unattributed, 0);
});

test('ribbons count licences, or the factories lent where the measure asks', () => {
  const { rows } = licencesAt(reply(), 25, () => true);
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
  assert.deepEqual(holderLicencesAt(renamed, holders, 25).rows.map(row => [row.licensor, row.licensee]), [['NEW', 'LIC'], ['NEW', 'LIC']]);
});

test('a refused licence roster is unknown, not zero licences', () => {
  assert.equal(rosterKnown(reply(), 15), true);
  assert.equal(rosterKnown({ known: [[0, 9, '1']] }, 15), false);
});
