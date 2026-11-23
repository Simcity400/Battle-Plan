import test from 'node:test';
import assert from 'node:assert/strict';
import { liveKey } from '../../charts/live.ts';
import { validateLeaseHistory, validateMarketReply } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import { agreementRows, agreementValues, amountOf } from './agreementsData.ts';
import type { CountryGroupReply, LeaseTerms } from '../../generated/dashboard';
import { countryMatch, membershipUnrecordedAt } from '../../dashboard/groups.ts';

const lease = (key: string, giver: string | null, receiver: string | null, extra: Partial<LeaseTerms> = {}): LeaseTerms => ({
  key, id: key.padStart(16, '0'), giver, receiver, ...(receiver === null && !('receiverMissing' in extra) ? { receiverMissing: 'Receiver not recorded' } : {}),
  revision: '0', activeSince: 10, lastDelivery: 0,
  monthly: [], percentage: [], once: [], delivered: [], sunk: [],
  fuelDailyRaw: '0', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0', ...extra });
const reply: MarketAt = { ...validateMarketReply({ branchId: '1', offers: null, contracts: null, purchasesMissing: null, unattributed: 0, warnings: [], icons: [],
  variants: [{ id: '6', hour: 50, known: true, token: 'tank', name: 'Tank', typeName: null, nameUnknown: null, icon: null, iconUnknown: null }],
  agreements: [
    lease('1', 'GER', 'FRA', { monthly: [{ variant: '6', name: null, amountRaw: '500000' }], delivered: [{ variant: '6', name: null, amountRaw: '1500000' }], fuelDailyRaw: '32768' }),
    lease('2', null, 'GER'),
  ] }), hour: 50 };
const history = validateLeaseHistory({ branchId: '1', warnings: [], icons: [], variants: [], agreements: [
  { terms: lease('3', 'GER', 'ITA', { once: [{ variant: '7', name: null, amountRaw: '100000' }] }), fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 1, lastHour: 30, endedBy: 31 },
  { terms: lease('4', 'GER', 'ENG', { fuelDailyRaw: '1' }), fuelSentRaw: '65536', fuelSunkRaw: '0', firstHour: 1, lastHour: 70, endedBy: 71 },
  { terms: lease('1', 'GER', 'FRA'), fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 1, lastHour: 20, endedBy: 21 },
] });
const ger = (tag: string) => tag === 'GER';

test('sent agreements at the hour, with their fuel as a row of its own, and ended ones from the history before the hour', () => {
  const { rows, unattributed } = agreementRows(reply, history, 'sent', ger, true);
  assert.deepEqual(rows.map(row => [row.key, row.ended, row.at, row.partner]),
    [['1', false, 50, 'FRA'], ['1:fuel', false, 50, 'FRA'], ['3:1:30', true, 30, 'ITA'], ['1:1:20', true, 20, 'FRA']],
    'one ending after the hour is not ended yet, while an earlier ended lifetime with the same key remains visible by endedBy');
  assert.equal(unattributed, 1, 'a giver with no tag may be GER');
  assert.deepEqual(rows[0].items.map(item => [item.name, item.schedule, item.delivered]), [['Tank', 'Monthly 5', 1500000n]]);
  assert.equal(rows[1].items[0].schedule, 'Daily 1');
  assert.deepEqual(agreementRows(reply, history, 'sent', ger, false).rows.map(row => row.key), ['1', '1:fuel']);
  assert.deepEqual(agreementRows(reply, history, 'received', ger, true).rows.map(row => row.key), ['2'], 'an active agreement with empty pools and zero fuel still renders a row');
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
  assert.deepEqual([ended.rows.map(row => row.key), ended.membership], [['1', '1:fuel', '1:1:20'], GAP], 'one ended at an hour not recorded');
  const recorded = read({ firstHour: 110, lastHour: 120 }, 50);
  assert.deepEqual([recorded.rows.map(row => row.key), recorded.membership], [['1', '1:fuel', '3:1:30', '1:1:20'], null]);
});

test('ended agreements with unrecorded pools, dates or fuel keep their missing reasons and never become zero', () => {
  const partialHistory = validateLeaseHistory({ branchId: '1', warnings: [], icons: [], variants: [], agreements: [{
    terms: lease('9', 'GER', null, {
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
    endedBy: 21,
  }] });
  const { rows } = agreementRows({ ...reply, agreements: [] }, partialHistory, 'sent', ger, true);
  assert.deepEqual(rows.map(row => row.key), ['9:1:20', '9:1:20:fuel']);
  assert.equal(rows[0].partnerMissing, "Receiver not recorded at the agreement's last read");
  assert.equal(rows[0].since, null);
  assert.equal(rows[0].sinceMissing, "Start date not recorded at the agreement's last read");
  assert.equal(rows[0].last, null);
  assert.equal(rows[0].lastMissing, "Last delivery date not recorded at the agreement's last read");
  assert.equal(rows[0].items[0].scheduleMissing, "Monthly: Could not be decoded at the agreement's last read");
  assert.equal(rows[0].items[0].deliveredMissing, "Not recorded at the agreement's last read");
  assert.equal(rows[1].items[0].delivered, null);
  assert.equal(rows[1].items[0].deliveredMissing, "Fuel sent not recorded at the agreement's last read");
  assert.equal(rows[1].items[0].lost, 0n);
  const live = { read: () => ({ get: () => undefined, missing: () => 'Not read yet' }), subscribe: () => () => {} };
  const values = agreementValues(rows, live).read();
  assert.equal(values.get(liveKey('9:1:20:fuel', 'delivered')), undefined);
  assert.equal(values.missing(liveKey('9:1:20:fuel', 'delivered')), "Fuel sent not recorded at the agreement's last read");
  assert.equal(values.get(liveKey('9:1:20:fuel', 'lost')), 0n);
});

test('endedBy gates ended rows, reused keys keep distinct deliveries, start hour 0 stays known, and null fields require reasons', () => {
  const delayed = validateLeaseHistory({ branchId: '1', warnings: [], icons: [], variants: [], agreements: [{
    terms: lease('12', 'GER', 'ITA', {
      activeSince: 0,
      monthly: [{ variant: '6', name: null, amountRaw: '500000' }],
      percentage: null,
      percentageMissing: "Could not be decoded at the agreement's last read",
      delivered: [{ variant: '6', name: null, amountRaw: '900719925474099300000' }],
    }),
    fuelSentRaw: '0',
    fuelSunkRaw: '0',
    firstHour: 1,
    lastHour: 30,
    endedBy: 60,
  }] });
  assert.deepEqual(agreementRows({ ...reply, hour: 50, agreements: [] }, delayed, 'sent', ger, true).rows, [],
    'not shown as ended before endedBy');
  const shown = agreementRows({ ...reply, hour: 60, agreements: [] }, delayed, 'sent', ger, true).rows;
  assert.equal(shown.length, 1);
  assert.equal(shown[0].since, 0, 'recorded start hour 0 stays 0, never missing');
  assert.equal(shown[0].sinceMissing, null);
  assert.equal(shown[0].last, null, 'lastDelivery 0 remains no-delivery sentinel');
  assert.equal(shown[0].lastMissing, null);
  assert.equal(shown[0].items[0].schedule, 'Monthly 5');
  assert.equal(shown[0].items[0].scheduleMissing, "Production share: Could not be decoded at the agreement's last read");
  const live = { read: () => ({ get: () => undefined, missing: () => 'Not read yet' }), subscribe: () => () => {} };
  const exact = agreementValues(shown, live).read().get(liveKey('12:1:30', 'delivered'));
  assert.equal(exact, 900719925474099300000n);
  assert.equal(amountOf(shown[0], exact as bigint), '9007199254740993');

  const reused = validateLeaseHistory({ branchId: '1', warnings: [], icons: [], variants: [], agreements: [
    { terms: lease('77', 'GER', 'FRA', { delivered: [{ variant: '6', name: null, amountRaw: '100000' }] }), fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 100, lastHour: 104, endedBy: 110 },
    { terms: lease('77', 'GER', 'FRA', { delivered: [{ variant: '6', name: null, amountRaw: '900000' }] }), fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 115, lastHour: 120, endedBy: 121 },
  ] });
  const duringSecond = agreementRows({ ...reply, hour: 116, agreements: [lease('77', 'GER', 'FRA', { delivered: [{ variant: '6', name: null, amountRaw: '500000' }] })] }, reused, 'sent', ger, true).rows;
  assert.deepEqual(duringSecond.map(row => [row.key, row.ended]), [['77', false], ['77:100:104', true]],
    'while a second agreement stands with the same key, the earlier ended lifetime stays visible');
  const afterBoth = agreementRows({ ...reply, hour: 125, agreements: [] }, reused, 'sent', ger, true).rows;
  assert.deepEqual(afterBoth.map(row => row.key), ['77:100:104', '77:115:120']);
  const bothValues = agreementValues(afterBoth, live).read();
  assert.equal(bothValues.get(liveKey('77:100:104', 'delivered')), 100000n);
  assert.equal(bothValues.get(liveKey('77:115:120', 'delivered')), 900000n);

  const zeroSchedule: MarketAt = { ...reply, agreements: [lease('20', 'GER', 'FRA'), lease('21', 'GER', 'ITA'), lease('22', 'GER', 'ENG')] };
  const wideFuel = 9007199254740993n * 32768n;
  const liveFuel = {
    get: (key: string) => key === liveKey('20', 'fuelSent') ? Number(wideFuel) : key === liveKey('20', 'fuelLost') ? 0 : key === liveKey('22', 'fuelSent') || key === liveKey('22', 'fuelLost') ? 0 : undefined,
    raw: (key: string) => key === liveKey('20', 'fuelSent') ? wideFuel : key === liveKey('20', 'fuelLost') ? 0n : key === liveKey('22', 'fuelSent') || key === liveKey('22', 'fuelLost') ? 0n : undefined,
    missing: () => 'Not recorded',
  };
  const fuelRows = agreementRows(zeroSchedule, undefined, 'sent', ger, false, () => null, liveFuel).rows;
  assert.deepEqual(fuelRows.map(row => row.key), ['20:fuel', '21:fuel', '22'], 'empty-pool zero-fuel standing agreement stays visible as its own row');
  assert.equal(fuelRows[0].items[0].delivered, wideFuel, 'live fuel above 2^53 stays exact via raw()');
  assert.equal(fuelRows[1].items[0].deliveredMissing, 'Not recorded');
  const fuelLive = agreementValues(fuelRows, { read: () => liveFuel, subscribe: () => () => {} }).read();
  assert.equal(fuelLive.get(liveKey('20:fuel', 'delivered')), wideFuel);
  assert.equal(fuelLive.raw?.(liveKey('20:fuel', 'delivered')), wideFuel);
  assert.equal(amountOf(fuelRows[0], fuelLive.raw!(liveKey('20:fuel', 'delivered'))!), '9007199254740993');

  const unattributedReply: MarketAt = { ...reply, agreements: [lease('30', 'GER', null, { receiverMissing: 'Receiver not recorded at this hour' })] };
  const received = agreementRows(unattributedReply, undefined, 'received', tag => tag === 'FRA', false);
  assert.deepEqual([received.rows.length, received.unattributedActive, received.unattributedEnded, received.unattributed], [0, 1, 0, 1],
    'unread standing receiver counts as unattributedActive');

  const endedHistory = validateLeaseHistory({
    branchId: '1', variants: [], icons: [], warnings: [],
    agreements: [{
      terms: lease('31', 'GER', null, { receiverMissing: 'Receiver not recorded at the agreement\'s last read' }),
      fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 10, lastHour: 30, endedBy: 31,
    }],
  });
  const endedOnly = agreementRows({ ...reply, agreements: [] }, endedHistory, 'received', tag => tag === 'FRA', true);
  assert.deepEqual([endedOnly.rows.length, endedOnly.unattributedActive, endedOnly.unattributedEnded, endedOnly.unattributed], [0, 0, 1, 1],
    'unread ended receiver never inflates unattributedActive');

  assert.throws(() => validateLeaseHistory({
    branchId: '1', variants: [], icons: [], warnings: [],
    agreements: [{
      terms: { ...lease('32', 'GER', 'FRA'), receiver: null },
      fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 10, lastHour: 30, endedBy: 31,
    }],
  }), 'null receiver without receiverMissing is rejected');
  assert.throws(() => validateLeaseHistory({
    branchId: '1', variants: [], icons: [], warnings: [],
    agreements: [{
      terms: { ...lease('33', 'GER', 'FRA'), activeSince: null },
      fuelSentRaw: '0', fuelSunkRaw: '0', firstHour: 10, lastHour: 30, endedBy: 31,
    }],
  }), 'null activeSince without activeSinceMissing is rejected');
  assert.throws(() => validateLeaseHistory({
    branchId: '1', variants: [], icons: [], warnings: [],
    agreements: [{
      terms: lease('34', 'GER', 'FRA'),
      fuelSentRaw: null, fuelSunkRaw: '0', firstHour: 10, lastHour: 30, endedBy: 31,
    }],
  }), 'null fuelSentRaw without fuelSentRawMissing is rejected');
});
