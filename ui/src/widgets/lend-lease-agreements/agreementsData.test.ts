import test from 'node:test';
import assert from 'node:assert/strict';
import { liveKey } from '../../charts/live.ts';
import { validateLeaseHistory, validateMarketReply } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import { agreementRows, agreementValues, amountOf } from './agreementsData.ts';
import type { CountryGroupReply, LeaseTerms } from '../../generated/dashboard';
import { countryMatch, membershipUnrecordedAt } from '../../dashboard/groups.ts';

const lease = (key: string, giver: string | null, receiver: string | null, extra: Partial<LeaseTerms> = {}): LeaseTerms => ({
  key, id: key.padStart(16, '0'), giver, receiver, revision: '0', activeSince: 10, lastDelivery: 0, monthly: [], percentage: [], once: [], delivered: [], sunk: [],
  fuelDailyRaw: '0', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0', ...extra });
const reply: MarketAt = { ...validateMarketReply({ branchId: '1', offers: null, contracts: null, purchasesMissing: null, unattributed: 0, warnings: [], icons: [],
  variants: [{ id: '6', hour: 50, known: true, token: 'tank', name: 'Tank', typeName: null, nameUnknown: null, icon: null, iconUnknown: null }],
  agreements: [
    lease('1', 'GER', 'FRA', { monthly: [{ variant: '6', name: null, amountRaw: '500000' }], delivered: [{ variant: '6', name: null, amountRaw: '1500000' }], fuelDailyRaw: '32768' }),
    lease('2', null, 'GER'),
  ] }), hour: 50 };
const history = validateLeaseHistory({ branchId: '1', warnings: [], icons: [], variants: [], agreements: [
  { terms: lease('3', 'GER', 'ITA', { once: [{ variant: '7', name: null, amountRaw: '100000' }] }), fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 1, lastHour: 30 },
  { terms: lease('4', 'GER', 'ENG', { fuelDailyRaw: '1' }), fuelSentRaw: '65536', fuelSunkRaw: '0', firstHour: 1, lastHour: 70 },
  { terms: lease('1', 'GER', 'FRA'), fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 1, lastHour: 20 },
] });
const ger = (tag: string) => tag === 'GER';

test('sent agreements at the hour, with their fuel as a row of its own, and ended ones from the history before the hour', () => {
  const { rows, unattributed } = agreementRows(reply, history, 'sent', ger, true);
  assert.deepEqual(rows.map(row => [row.key, row.ended, row.at, row.partner]), [['1', false, 50, 'FRA'], ['1:fuel', false, 50, 'FRA'], ['3', true, 30, 'ITA']],
    'one ending after the hour is not ended yet, and a key standing now is not listed again as ended');
  assert.equal(unattributed, 1, 'a giver with no tag may be GER');
  assert.deepEqual(rows[0].items.map(item => [item.name, item.schedule, item.delivered]), [['Tank', 'Monthly 5', 1500000n]]);
  assert.equal(rows[1].items[0].schedule, 'Daily 1');
  assert.deepEqual(agreementRows(reply, history, 'sent', ger, false).rows.map(row => row.key), ['1', '1:fuel']);
  assert.deepEqual(agreementRows(reply, history, 'received', ger, true).rows.map(row => row.key), [], 'the agreement GER receives has a giver with no tag');
});

test('Delivered and Lost paint an active agreement\'s fuel from its live cells and every other row as read', () => {
  const { rows } = agreementRows(reply, history, 'sent', ger, true);
  const live = { read: () => ({ get: (key: string) => key === liveKey('1', 'fuelSent') ? 98304 : undefined, missing: () => 'Not read yet' }), subscribe: () => () => {} };
  const values = agreementValues(rows, live).read();
  assert.equal(values.get(liveKey('1:fuel', 'delivered')), 98304);
  assert.equal(values.get(liveKey('1:fuel', 'lost')), undefined, 'a fuel loss not read is missing, never zero');
  assert.equal(values.missing(liveKey('1:fuel', 'lost')), 'Not read yet');
  assert.equal(values.get(liveKey('1', 'delivered')), 1500000);
  assert.equal(amountOf(rows[1], 98304n), '3');
});

const humans = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
const gapGroups = (tags: readonly string[], gap = { firstHour: 110, lastHour: 120 }): CountryGroupReply => ({ branchId: '1', warnings: [],
  humans: tags.map(tag => ({ tag, firstHour: 0, lastHour: 200 })), factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
  countryIdentities: tags.map((tag, slot) => ({ tag, identities: [{ firstHour: 0, lastHour: 200, slot, countryId: slot + 1 }] })),
  unobserved: { humans: [gap], factions: [], wars: [] } });
const GAP = 'Group membership is unavailable at this time.';

test('an agreement left out where the chosen group’s membership was not recorded at its hour is said, never no lend-lease', () => {
  const read = (gap: { firstHour: number; lastHour: number }, hour: number) => {
    const groups = gapGroups(['GER', 'FRA', 'ITA', 'ENG'], gap);
    return agreementRows({ ...reply, hour }, history, 'sent', countryMatch(humans, [], groups, 200).matches, true, at => membershipUnrecordedAt(humans, groups, at));
  };
  const unknown = read({ firstHour: 110, lastHour: 120 }, 115);
  assert.deepEqual([unknown.rows.filter(row => !row.ended).length, unknown.membership], [0, GAP], 'the standing agreements at an hour not recorded');
  const ended = read({ firstHour: 25, lastHour: 35 }, 50);
  assert.deepEqual([ended.rows.map(row => row.key), ended.membership], [['1', '1:fuel'], GAP], 'one ended at an hour not recorded');
  const recorded = read({ firstHour: 110, lastHour: 120 }, 50);
  assert.deepEqual([recorded.rows.map(row => row.key), recorded.membership], [['1', '1:fuel', '3'], null]);
});
