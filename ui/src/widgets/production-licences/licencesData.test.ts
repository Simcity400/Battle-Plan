import assert from 'node:assert/strict';
import test from 'node:test';
import { countryLicences, licencePairs, licencesAt, pickedTag, slotOf, validateLicences } from './licencesData.ts';
import type { ProductionLicencesReply } from '../../generated/dashboard';

const SLOTS: Record<string, number> = { GER: 7, ITA: 8, RSI: 8 };
const party = (tag: string | null) => ({ slot: tag === null ? null : SLOTS[tag], tag });
const reply: ProductionLicencesReply = { branchId: '1', missing: null, warnings: [], licences: [
  { key: '11:0', licensor: party('ITA'), licensee: party('GER'), lent: 2, required: 1, since: 1000, designs: [{ variant: '300', name: 'Fiat' }] },
  { key: '11:1', licensor: party('ITA'), licensee: party('GER'), lent: 0, required: 0, since: 1010, designs: [{ variant: '301', name: null }] },
  { key: '9002:0', licensor: party(null), licensee: party('ITA'), lent: 1, required: 0, since: 1200, designs: [] },
] };

test('licences a kept country grants or holds are shown, and those a party without a country may hold are counted, never dropped silently', () => {
  const keepGer = (tag: string) => tag === 'GER';
  const { rows, unattributed } = licencesAt(reply, 1210, keepGer);
  assert.deepEqual(rows.map(row => row.key), ['11:0', '11:1']);
  assert.equal(unattributed, 1);
  assert.deepEqual(licencesAt(reply, 1210, tag => tag === 'ITA').rows.map(row => row.key), ['11:0', '11:1', '9002:0']);
});

test('pairs sum licences or lent factories, and a country lists what it grants and holds', () => {
  const pairs = licencePairs(licencesAt(reply, 1210, () => true).rows, 'factories');
  assert.deepEqual(pairs.map(pair => [pair.licensor, pair.licensee, pair.width]), [['ITA', 'GER', 2], [null, 'ITA', 1]]);
  assert.deepEqual(licencePairs(licencesAt(reply, 1210, () => true).rows, 'licences').map(pair => pair.width), [2, 1]);
  assert.deepEqual(countryLicences(reply, { slot: slotOf(reply, 'ITA'), tag: 'ITA' }).rows.map(row => row.key), ['11:0', '11:1', '9002:0']);
  assert.equal(countryLicences(reply, { slot: slotOf(reply, 'GER'), tag: 'GER' }).unattributed, 1);
});

test('a picked country is kept by its slot when it holds another tag at the drill day', () => {
  const picked = { slot: slotOf(reply, 'ITA'), tag: 'ITA' };
  const later: ProductionLicencesReply = { ...reply, licences: reply.licences.map(licence => ({ ...licence,
    licensor: licence.licensor.tag === 'ITA' ? party('RSI') : licence.licensor, licensee: licence.licensee.tag === 'ITA' ? party('RSI') : licence.licensee })) };
  assert.deepEqual(countryLicences(later, picked).rows.map(row => row.key), ['11:0', '11:1', '9002:0']);
  assert.equal(pickedTag(later, picked), 'RSI');
  assert.equal(pickedTag(later, { slot: 99, tag: 'XXX' }), null, 'a country no licence names has no tag here');
});

test('a reply with a malformed licence is refused', () => {
  assert.deepEqual(validateLicences(reply), reply);
  assert.throws(() => validateLicences({ ...reply, licences: [{ ...reply.licences[0], lent: '2' }] }), /Invalid production licences/);
});
