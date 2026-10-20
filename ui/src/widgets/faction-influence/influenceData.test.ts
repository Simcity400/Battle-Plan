import assert from 'node:assert/strict';
import test from 'node:test';
import type { FactionDetailReply, FactionInfluenceAccount, FactionInfluenceReply } from '../../generated/dashboard';
import { accountHolders, influenceShareRuns, memberAccounts, memberShareRuns, rankingRows } from './influenceData.ts';
import { shareStack } from '../../charts/share.ts';

const account = (key: string, member: [number, number][], runs: [number, number, string | null][]): FactionInfluenceAccount => ({ key, tag: `mod:${key}`, slot: 1, countryId: Number(key.charCodeAt(0)), retained: [],
  member: member.map(([firstHour, lastHour]) => ({ firstHour, lastHour })),
  runs: runs.map(([firstHour, lastHour, shareRaw]) => ({ firstHour, lastHour, shareRaw, startsSegment: false })) });

test('stacked shares hold each member’s recorded share; an unknown share leaves a gap, a late joiner enters at its join', () => {
  const runs = influenceShareRuns([
    account('a', [[0, 99]], [[0, 49, '60000'], [50, 99, '50000']]),
    account('b', [[0, 99]], [[0, 9, null], [10, 99, '40000']]),
    account('c', [[50, 99]], [[0, 99, '10000']]),
  ]);
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.counts]), [
    [10, 49, [60000, 40000, 0, 0]], [50, 99, [50000, 40000, 10000, 0]]], 'hours 0-9 are unknown for a member, so no column; c counts only as a member');
  assert.equal(runs[0].startsSegment, true, 'the first column after a gap starts a segment');
  assert.equal(runs[1].startsSegment, undefined);
});

test('a lone member keeps its recorded share of the whole through the chart adapter', () => {
  const runs = influenceShareRuns([account('a', [[0, 9]], [[0, 9, '25000']])]);
  assert.deepEqual(runs[0].counts, [25000, 75000], 'the share no member holds is its own band');
  assert.ok(Math.abs(shareStack(runs, 2).edges[0]![0].high - 25) < 1e-9, 'a lone 25% is not renormalised to 100%');
});

test('a member’s drill pairs its share with the rest, and the ranking holds current members only', () => {
  const drill = memberShareRuns(account('a', [[0, 20]], [[0, 99, '25500'], [100, 120, null]]));
  assert.deepEqual(drill.map(run => run.counts), [[25500, 74500]]);
  assert.ok(Math.abs(shareStack(drill, 2).edges[0]![0].high - 25.5) < 1e-9, 'the real chart adapter keeps 25.5% of the whole');
  assert.deepEqual(memberAccounts({ accounts: [account('a', [], []), account('b', [[0, 1]], [])] } as unknown as FactionInfluenceReply).map(item => item.key), ['b']);
  const data = { accounts: [{ ordinal: 0, tag: 'A', currentMember: true }, { ordinal: 1, tag: 'B', currentMember: false }] } as unknown as FactionDetailReply;
  assert.deepEqual(rankingRows(data).map(row => row.tag), ['A'], 'a retained account of a departed country is not ranked');
});

test('two accounts of one reused tag keep their own names and colours throughout history', async () => {
  const { holderLooks } = await import('../../dashboard/holderLooks.ts');
  const { nameKey } = await import('../../dashboard/countryFacts.ts');
  const old = { ...account('a', [[0, 23]], [[0, 23, '50000']]), tag: 'mod:Ω' };
  const replacement = { ...account('b', [[24, 99]], [[24, 99, '40000']]), tag: 'mod:Ω', slot: 2 };
  const holders = accountHolders([old, replacement]);
  assert.deepEqual(holders, [{ key: 'a', tag: 'mod:Ω', hour: 23 }, { key: 'b', tag: 'mod:Ω', hour: 99 }], 'each account is read at an hour it was itself a member');
  const styles = new Map([[nameKey(23, 'mod:Ω'), { tag: 'mod:Ω', name: 'The earlier holder', color: [200, 10, 10] as [number, number, number] }],
    [nameKey(99, 'mod:Ω'), { tag: 'mod:Ω', name: 'The later holder', color: [10, 10, 200] as [number, number, number] }]]);
  const looks = holderLooks(holders, styles, { surface: '#ffffff', muted: '#888888', countryColours: 'raw' });
  assert.deepEqual([looks.name('a'), looks.name('b')], ['The earlier holder', 'The later holder']);
  assert.deepEqual([looks.colour('a'), looks.colour('b')], ['#c80a0a', '#0a0ac8'], 'the current holder’s colour never paints the earlier account');
});
