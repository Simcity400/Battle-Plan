import test from 'node:test';
import assert from 'node:assert/strict';
import { navalDetailTooltip, navalFleetDescription, validateNavalBattleDetails } from './navalBattleDetails.ts';
import type { NavalBattleDetail } from '../../generated/dashboard';

const detail: NavalBattleDetail = { id: 'result:15', hour: 43, rosterHour: 8,
  fleets: [{ names: ['Recorded fleet'], unknown: null }, { names: [], unknown: 'Fleet name not recorded' }],
  namedShips: [{ sinkHour: 9, knownAt: 19, name: 'Recorded ship', className: 'Mod ship class', sinkingCause: { kind: 'notRecorded', reason: 'sinking cause was not recorded' } }], shipRead: 43, unavailable: null };
test('fleet and sinking details preserve names and sinking dates without supplying dates', () => {
  assert.equal(validateNavalBattleDetails({ rows: [detail], warnings: [] }).rows[0], detail);
  assert.equal(navalFleetDescription(detail, { pending: false }), 'Recorded fleet v Fleet names not recorded');
  const rows = navalDetailTooltip(detail);
  assert.ok(rows.some(row => row.label === 'Recorded ship' && row.value === 'Mod ship class'));
  assert.ok(!rows.some(row => row.label === 'Fleet roster read'));
  assert.ok(!rows.some(row => row.label === 'Recorded at'));
  assert.ok(rows.some(row => row.label === 'Sunk'));
});

test('a row whose details are not held says so as its subtitle: loading, the failure, or not answered; never nothing (run 12: every subtitle gone)', () => {
  assert.equal(navalFleetDescription(undefined, { pending: true }), 'Loading fleets…');
  assert.equal(navalFleetDescription(undefined, { pending: false, error: new Error('private backend error') }), 'Fleets couldn’t load');
  assert.equal(navalFleetDescription(undefined, { pending: false }), 'Fleets not read for this battle');
});

test('detail failures and missing fleets hide backend reasons without claiming no losses', () => {
  assert.deepEqual(navalDetailTooltip(undefined, new Error('private backend error')), [
    { label: 'Couldn’t load battle details.', detail: 'private backend error', tone: 'muted', divider: true },
  ]);
  assert.deepEqual(navalDetailTooltip(undefined), [{ label: 'Loading battle details…', detail: undefined, tone: 'muted', divider: true }]);
  const rows = navalDetailTooltip({ ...detail, fleets: [{ names: [], unknown: 'native field refused' }, { names: [], unknown: null }],
    shipRead: null, namedShips: [], unavailable: 'private table unavailable' });
  assert.ok(rows.some(row => row.label === 'Fleet details unavailable.'));
  assert.ok(rows.some(row => row.label === 'No fleets'));
  assert.equal(navalFleetDescription({ ...detail, fleets: [{ names: ['Fleet 1'], unknown: null }, { names: [], unknown: null }] }, { pending: false }),
    'Fleet 1 v No fleets');
  assert.ok(rows.some(row => row.label === 'Named ship losses unavailable.'));
  assert.ok(!rows.some(row => row.label === 'No named ship losses recorded.'));
  assert.doesNotMatch(JSON.stringify(rows), /private|native field/);
});
test('future ship reads and mismatched repeated detail identities refuse', () => {
  assert.throws(() => validateNavalBattleDetails({ rows: [{ ...detail, namedShips: [{ ...detail.namedShips[0], knownAt: 44 }] }], warnings: [] }));
  assert.throws(() => validateNavalBattleDetails({ rows: [detail, detail], warnings: [] }));
  assert.throws(() => validateNavalBattleDetails({ rows: [{ ...detail, rosterHour: 44 }], warnings: [] }));
});
test('all recorded fleet and ship names remain in tooltip content without a chosen cap', () => {
  const many = { ...detail, fleets: [{ names: Array.from({ length: 5000 }, (_, i) => `Fleet ${i}`), unknown: null }, detail.fleets[1]],
    namedShips: Array.from({ length: 5000 }, (_, i) => ({ ...detail.namedShips[0], name: `Ship ${i}` })) };
  const valid = validateNavalBattleDetails({ rows: [many], warnings: [] });
  const rows = navalDetailTooltip(valid.rows[0]);
  assert.ok(rows.some(row => row.label === 'Fleet 4999'));
  assert.ok(rows.some(row => row.label === 'Ship 4999'));
});
