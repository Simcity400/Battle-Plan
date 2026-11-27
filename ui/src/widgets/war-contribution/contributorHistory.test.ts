import assert from 'node:assert/strict';
import test from 'node:test';
import type { WarContributionHistoryReply, WarHistoryContributor } from '../../generated/dashboard';
import { deriveTimeChartTooltip } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import { detailAt, historyChart, permyriad, readingText } from './contributorHistory.ts';
import cases from './permyriad.test.json' with { type: 'json' };

const person: WarHistoryContributor = {
  slot: 1, id: 1, membership: [[103, 106, 0, 'ONE']],
  amounts: [[[103, '37000']], [[103, '19']], [[103, '0']], [], [], [[103, '137']], [], [], [], []],
  partialCasualties: [{ firstHour: 103, lastHour: 104 }],
};

const history = {
  war: {
    id: '0', firstHour: 100, lastHour: 107, startedBy: true, endedBy: true,
    observations: [{ firstHour: 103, lastHour: 103, viewpointTag: 'ONE', viewpointSide: ['ONE', 'TWO'], opposingSide: ['THR'], contributionGroup: [['ONE'], ['THR']],
      attackersAreViewpoint: true, representative: null, name: { known: true, text: 'Fixture War', reason: null, key: null } }],
    participants: [{ tag: 'ONE', side: 'attacker', firstHour: 103, lastHour: 106, joinedBy: false, leftBy: false }],
    capitulations: [{ tag: 'ONE', hour: 105 }],
  },
  measureKeys: ['participation', 'equipmentDamage', 'occupation', 'airDamage', 'shipDamage', 'casualties', 'strategicBombing', 'convoyDamage', 'lendLeaseSent', 'lendLeaseReceived'],
  contributors: [person],
  sides: [{
    totals: [[[103, '100000']], [[103, '39']], [[103, '0']], [], [], [[103, '279']], [], [], [], []],
    missing: [],
  }, {
    totals: [[], [[103, '41']], [[103, '0']], [], [], [[103, '111']], [], [], [], []],
    missing: [{ firstHour: 104, lastHour: 104, reason: 'Fixture opposing ledger unread' }],
  }],
  gaps: [{ firstHour: 102, lastHour: 102, reason: 'Fixture pause' }, { firstHour: 108, lastHour: 109, reason: 'Fixture ending pause' }],
  warnings: [],
} satisfies WarContributionHistoryReply;

test('share arithmetic uses the same exact vectors as the Rust snapshot', () => {
  for (const [part, whole, expected] of cases as [string, string, number | null][]) {
    assert.equal(permyriad(BigInt(part), BigInt(whole)), expected === null ? null : BigInt(expected), `${part} of ${whole}`);
  }
  assert.equal(permyriad(null, 100n), null);
  assert.equal(permyriad(100n, null), null);
});

test('held history distinguishes absence, partial casualties, a missing side and a pause', () => {
  const read = (hour: number) => detailAt(history, person, 5, hour);
  assert.deepEqual(read(100)[0], { kind: 'notApplicable', reason: 'This country was not in the war' });
  assert.deepEqual(read(102)[0], { kind: 'notRecorded', reason: 'Fixture pause' });
  const parts = read(103);
  assert.deepEqual(parts.map(part => part.kind), ['partial', 'partial', 'partial']);
  assert.match(readingText(parts[0]!, 0), /partial.*not attributed/i);
  assert.deepEqual(read(104)[2], { kind: 'notRecorded', reason: 'Fixture opposing ledger unread' });
  assert.equal(read(104)[1]!.kind, 'partial');
  assert.equal(read(105)[0]!.kind, 'known');
  assert.equal(read(107)[0]!.kind, 'notApplicable');
  const zeroShares = detailAt(history, person, 2, 105);
  assert.deepEqual(zeroShares.map(value => value.kind), ['zero', 'notApplicable', 'notApplicable']);
  assert.match(readingText(zeroShares[1]!, 1), /recorded total for this side is zero/i);
  const chart = historyChart(history, [person], 'casualties', 2, 'white');
  assert.equal(chart.hour, 109);
  assert.equal(chart.series[0]?.points?.find(point => point.hour === 100)?.value, null);
  assert.equal(chart.series[0]?.points?.find(point => point.hour === 104)?.value, null);
  assert.deepEqual(chart.markers?.map(marker => marker.hour), [103, 106, 105]);
  const tipAt = (chart: ReturnType<typeof historyChart>, hour: number) => deriveTimeChartTooltip({ ...chart, hoverDetail: chart.detailAt(hour) },
    layoutTimeChart(chart, 700, 230, 'sans-serif', text => text.length * 7), `plot:${hour}`)?.payload?.items?.[0]?.value ?? '';
  assert.match(tipAt(chart, 103), /^35\.13% partial.*not attributed/i);
  assert.match(tipAt(chart, 104), /Fixture opposing ledger unread/);
  const layout = layoutTimeChart(chart, 700, 230, 'sans-serif', text => text.length * 7);
  const join = layout.markers.findIndex(item => item.marker.id === 'join-0');
  assert.ok(join >= 0);
  const marker = deriveTimeChartTooltip({ ...chart, hoverDetail: chart.detailAt(103) }, layout, `marker:${join}`)?.payload?.items;
  assert.match(marker?.[0]?.label ?? '', /ONE joined/);
  assert.match(marker?.[1]?.value ?? '', /^35\.13% Partial.*1 country on the war roster is outside.*not attributed/);
  const participation = historyChart(history, [person], 'participation', 2, 'white');
  assert.match(tipAt(participation, 100), /This country was not in the war/);
  assert.match(tipAt(participation, 103), /Participation is already a share of the side/);
});
