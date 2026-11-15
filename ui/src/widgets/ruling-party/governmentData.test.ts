import test from 'node:test';
import assert from 'node:assert/strict';
import type { LedgerGovernment } from './partyTypes';
import { governmentBadges, governmentCause, governmentEventRows, governmentRows } from './governmentData.ts';
import { partyBadges } from './partyData.ts';
import type { PartyHistoryReply } from '../../generated/dashboard';

const transition: LedgerGovernment = { country: 1, instance: 1, epoch: '1', sequence: '1', hour: 24, readHour: 26,
  beforeToken: 'mod_old', afterToken: 'mod_new', cause: 'CoupCompletion', covered: false };
const parties: PartyHistoryReply = { branchId: '1', tag: 'mod country', decimals: 5, parties: [], leaders: [], people: [],
  ideologyTypes: [], ideologies: [], partyNames: [], partyNameRuns: [], unobserved: [], warnings: [], elections: [26],
  ruling: [{ token: 'mod_old', firstHour: 24, lastHour: 25, startsSegment: true },
    { token: 'mod_new', firstHour: 26, lastHour: 30, startsSegment: false }] };

test('a native cause appears after its supplying read, at its execution date with explicit coverage', () => {
  assert.deepEqual(governmentBadges([transition], token => token, 25), []);
  const badge = governmentBadges([transition], token => token, 26)[0];
  assert.equal(badge.hour, 24);
  assert.match(badge.label, /mod_new took power from mod_old.*Coup.*Some changes may be missing/);
  assert.equal(governmentRows([transition], 25, 30, token => token).length, 0);
  assert.equal(governmentRows([transition], 26, 30, token => token)[0].value, 'Coup');
  assert.equal(governmentCause('not-a-native-enum'), 'Unknown cause');
  assert.ok(governmentEventRows({ ...transition, readHour: 49 }, token => token).some(row => row.value === 'Coup'));
});

test('only exact endpoint and supplying-read matches supersede census; elections remain observations', () => {
  assert.deepEqual(partyBadges(parties, token => token, false, [transition]).map(badge => badge.glyph), ['\u2713']);
  for (const unrelated of [{ ...transition, afterToken: 'other' }, { ...transition, beforeToken: null }, { ...transition, readHour: 27 }]) {
    const badges = partyBadges(parties, token => token, false, [unrelated]);
    assert.equal(badges.filter(badge => badge.glyph === '\u21c6').length, 1);
    assert.equal(badges.find(badge => badge.glyph === '\u21c6')?.label.includes('Election'), false);
  }
  assert.equal(governmentRows([{ ...transition, beforeToken: null, afterToken: null }], 26, 30, token => token)[1].value,
    'Not identified / Not identified');
});

