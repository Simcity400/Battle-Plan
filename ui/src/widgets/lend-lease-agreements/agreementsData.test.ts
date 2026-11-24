import test from 'node:test';
import assert from 'node:assert/strict';
import { liveKey } from '../../charts/live.ts';
import { validateLeaseHistory, validateMarketReply } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import { agreementRows, agreementValues, amountOf } from './agreementsData.ts';
import type { CountryGroupReply, EndedLeaseTerms, LeaseTerms } from '../../generated/dashboard';
import { countryMatch, membershipUnrecordedAt } from '../../dashboard/groups.ts';

const lease = (key: string, giver: string | null, receiver: string | null, extra: Partial<LeaseTerms> = {}): LeaseTerms => ({
  key, id: key.padStart(16, '0'), giver, receiver, revision: '0', activeSince: 10, lastDelivery: 0, monthly: [], percentage: [], once: [], delivered: [], sunk: [],
  fuelDailyRaw: '0', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0', ...extra });
const endedTerms = (key: string, giver: string | null, receiver: string | null, extra: Partial<EndedLeaseTerms> = {}): EndedLeaseTerms => ({
  key, id: key.padStart(16, '0'), giver, receiver, revision: '0', activeSince: 10, lastDelivery: 0, monthly: [], percentage: [], once: [], delivered: [], sunk: [],
  fuelDailyRaw: '0', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0', ...extra });
const reply: MarketAt = { ...validateMarketReply({ branchId: '1', offers: null, contracts: null, purchasesMissing: null, unattributed: 0, warnings: [], icons: [],
  variants: [{ id: '6', hour: 50, known: true, token: 'tank', name: 'Tank', typeName: null, nameUnknown: null, icon: null, iconUnknown: null }],
  agreements: [
    lease('1', 'GER', 'FRA', { monthly: [{ variant: '6', name: null, amountRaw: '500000' }], delivered: [{ variant: '6', name: null, amountRaw: '1500000' }], fuelDailyRaw: '32768' }),
    lease('2', null, 'GER'),
  ] }), hour: 50 };
const history = validateLeaseHistory({ branchId: '1', warnings: [], icons: [], variants: [], agreements: [
  { terms: endedTerms('3', 'GER', 'ITA', { once: [{ variant: '7', name: null, amountRaw: '100000' }] }), fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 1, lastHour: 30 },
  { terms: endedTerms('4', 'GER', 'ENG', { fuelDailyRaw: '1' }), fuelSentRaw: '65536', fuelSunkRaw: '0', firstHour: 1, lastHour: 70 },
  { terms: endedTerms('1', 'GER', 'FRA'), fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 1, lastHour: 20 },
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
  assert.equal(values.get(liveKey('1', 'delivered')), 1500000n);
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

test('ended agreements with unrecorded pools, dates, fuel or party tags keep their missing reasons and never become zero', () => {
  const partialHistory = validateLeaseHistory({ branchId: '1', warnings: [], icons: [], variants: [], agreements: [
    {
      terms: endedTerms('9', 'GER', null, {
        receiverMissing: "Receiver not recorded at the agreement's last read",
        activeSince: null,
        activeSinceMissing: "Start date not recorded at the agreement's last read",
        lastDelivery: null,
        lastDeliveryMissing: "Last delivery date not recorded at the agreement's last read",
        monthly: null,
        monthlyMissing: "Could not be decoded at the agreement's last read",
        delivered: null,
        deliveredMissing: "Not recorded at the agreement's last read",
        fuelDailyRaw: null,
        fuelDailyRawMissing: "Daily fuel not recorded at the agreement's last read",
      }),
      fuelSentRaw: null,
      fuelSentRawMissing: "Fuel sent not recorded at the agreement's last read",
      fuelSunkRaw: '0',
      firstHour: 1,
      lastHour: 20,
    },
    {
      terms: endedTerms('10', null, 'GER', {
        giverMissing: "Giver country 77 had no tag at the agreement's last read",
        monthly: [{ variant: '6', name: null, amountRaw: '100000' }],
      }),
      fuelSentRaw: '0',
      fuelSunkRaw: '0',
      firstHour: 1,
      lastHour: 20,
    },
  ] });
  const sent = agreementRows({ ...reply, agreements: [] }, partialHistory, 'sent', ger, true);
  assert.deepEqual(sent.rows.map(row => row.key), ['9', '9:fuel']);
  assert.equal(sent.unattributed, 0, 'ended rows with missing giver or receiver do not inflate unattributed');
  assert.equal(sent.rows[0].partnerMissing, "Receiver not recorded at the agreement's last read");
  assert.equal(sent.rows[0].receiverMissing, "Receiver not recorded at the agreement's last read");
  assert.equal(sent.rows[0].since, null);
  assert.equal(sent.rows[0].sinceMissing, "Start date not recorded at the agreement's last read");
  assert.equal(sent.rows[0].last, null);
  assert.equal(sent.rows[0].lastMissing, "Last delivery date not recorded at the agreement's last read");
  assert.equal(sent.rows[0].items[0].scheduleMissing, "Monthly: Could not be decoded at the agreement's last read");
  assert.equal(sent.rows[0].items[0].deliveredMissing, "Not recorded at the agreement's last read");
  assert.equal(sent.rows[1].items[0].delivered, null);
  assert.equal(sent.rows[1].items[0].deliveredMissing, "Fuel sent not recorded at the agreement's last read");
  assert.equal(sent.rows[1].items[0].lost, 0n);
  const live = { read: () => ({ get: () => undefined, missing: () => 'Not read yet' }), subscribe: () => () => {} };
  const values = agreementValues(sent.rows, live).read();
  assert.equal(values.get(liveKey('9:fuel', 'delivered')), undefined);
  assert.equal(values.missing(liveKey('9:fuel', 'delivered')), "Fuel sent not recorded at the agreement's last read");
  assert.equal(values.get(liveKey('9:fuel', 'lost')), 0n);

  const received = agreementRows({ ...reply, agreements: [] }, partialHistory, 'received', ger, true);
  assert.deepEqual(received.rows.map(row => row.key), ['10']);
  assert.equal(received.unattributed, 0, 'ended agreement with unread receiver is not counted as a possible incoming match');
  assert.equal(received.rows[0].giver, null);
  assert.equal(received.rows[0].giverMissing, "Giver country 77 had no tag at the agreement's last read");
  assert.equal(received.rows[0].partnerMissing, "Giver country 77 had no tag at the agreement's last read");
});

test('recorded activeSince 0 and lastDelivery 0, older-host replies, and validation of missing reasons', () => {
  const olderHostReply = validateLeaseHistory({
    branchId: '1', warnings: [], icons: [], variants: [],
    agreements: [
      {
        terms: {
          key: '12', id: '000000000000000c', giver: 'GER', receiver: null,
          revision: '0', activeSince: 0, lastDelivery: 0,
          monthly: [{ variant: '6', name: null, amountRaw: '500000' }],
          percentage: [], once: [],
          delivered: [{ variant: '6', name: null, amountRaw: '900719925474099300000' }],
          sunk: [],
          fuelDailyRaw: '0', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0',
        },
        fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 1, lastHour: 30,
      },
      {
        terms: {
          key: '13', id: '000000000000000d', giver: null, receiver: 'GER',
          revision: '0', activeSince: 5, lastDelivery: 25,
          monthly: [{ variant: '6', name: null, amountRaw: '100000' }],
          percentage: [], once: [], delivered: [], sunk: [],
          fuelDailyRaw: '0', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0',
        },
        fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 1, lastHour: 30,
      },
    ],
  });
  const shown = agreementRows({ ...reply, hour: 50, agreements: [] }, olderHostReply, 'sent', ger, true).rows;
  assert.equal(shown.length, 1);
  assert.equal(shown[0].since, 0, 'recorded start hour 0 stays 0, never missing');
  assert.equal(shown[0].sinceMissing, null);
  assert.equal(shown[0].last, null, 'lastDelivery 0 remains no-delivery sentinel');
  assert.equal(shown[0].lastMissing, null);
  assert.equal(shown[0].receiver, null);
  assert.equal(shown[0].receiverMissing, null, 'older host reply without receiverMissing is accepted');
  const live = { read: () => ({ get: () => undefined, missing: () => 'Not read yet' }), subscribe: () => () => {} };
  const exact = agreementValues(shown, live).read().get(liveKey('12', 'delivered'));
  assert.equal(exact, 900719925474099300000n);
  assert.equal(amountOf(shown[0], exact as bigint), '9007199254740993');

  const olderIncoming = agreementRows({ ...reply, hour: 50, agreements: [] }, olderHostReply, 'received', ger, true).rows;
  assert.equal(olderIncoming.length, 1);
  assert.equal(olderIncoming[0].giver, null);
  assert.equal(olderIncoming[0].giverMissing, null, 'older host reply without giverMissing is accepted');
  assert.equal(olderIncoming[0].last, 25);

  assert.throws(() => validateLeaseHistory({
    branchId: '1', variants: [], icons: [], warnings: [],
    agreements: [{
      terms: { ...endedTerms('32', 'GER', null), receiverMissing: '' },
      fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 10, lastHour: 30,
    }],
  }), 'empty receiverMissing string is rejected');
  assert.throws(() => validateLeaseHistory({
    branchId: '1', variants: [], icons: [], warnings: [],
    agreements: [{
      terms: { ...endedTerms('33', 'GER', 'FRA'), activeSince: null },
      fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 10, lastHour: 30,
    }],
  }), 'null activeSince without activeSinceMissing is rejected');
  assert.throws(() => validateLeaseHistory({
    branchId: '1', variants: [], icons: [], warnings: [],
    agreements: [{
      terms: endedTerms('34', 'GER', 'FRA'),
      fuelSentRaw: null, fuelSunkRaw: '0', firstHour: 10, lastHour: 30,
    }],
  }), 'null fuelSentRaw without fuelSentRawMissing is rejected');
});
