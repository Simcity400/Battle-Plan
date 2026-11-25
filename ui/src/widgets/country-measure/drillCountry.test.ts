import assert from 'node:assert/strict';
import test from 'node:test';
import { countryDrillTagAt } from '../../dashboard/countryIdentity.ts';
import type { BoundTrack } from '../../dashboard/model';
import { chooseDrillCountry, drillCountryCoverage, drillCountrySpans } from './drillCountry.ts';

const histories = [
  { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }, { firstHour: 110, lastHour: 120, slot: 2, countryId: 20 }] },
  { tag: 'Y', identities: [{ firstHour: 110, lastHour: 113, slot: 1, countryId: 10 }, { firstHour: 115, lastHour: 120, slot: 1, countryId: 10 }] },
];
const track = { branch: { id: 'b', firstHour: 100, lastHour: 120 }, source: { metadata: { campaignKey: 'campaign' } } } as BoundTrack;

test('saved native countries follow renames without a reused tag or an identity gap', () => {
  const selected = chooseDrillCountry(histories, track, 'X', 105)!;
  assert.deepEqual(selected, { campaign: 'campaign', branch: 'b', tag: 'X', originOffset: 5, slot: 1, countryId: 10 });
  const spans = drillCountrySpans(histories, selected, track)!;
  assert.equal(countryDrillTagAt(spans, 109), 'X');
  assert.equal(countryDrillTagAt(spans, 110), 'Y');
  assert.equal(countryDrillTagAt(spans, 114), null);
  assert.equal(countryDrillTagAt(spans, 120), 'Y');
  assert.deepEqual(drillCountryCoverage(spans), [{ firstHour: 100, lastHour: 113 }, { firstHour: 115, lastHour: 120 }]);
  assert.equal(drillCountrySpans(histories, { ...selected, countryId: 20 }, track), null);
  assert.equal(drillCountrySpans(histories, { ...selected, originOffset: 21 }, track), null);
  assert.equal(chooseDrillCountry(histories, track, 'Y', 114), null);
});
