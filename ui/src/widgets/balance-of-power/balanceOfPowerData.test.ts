import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView, type ArtResult } from '../../families/contract.ts';
import type { BalanceHistory, BalanceRecord, PoliticsHistoryReply, PoliticsStateReply } from '../../generated/dashboard';
import {
  SIDE_UNAVAILABLE,
  balanceAbsence,
  balanceAxis,
  chooseBalance,
  countryHolder,
  holderRecord,
  recordFor,
  sumWeeklyDrift,
  validateBalanceList,
  validateCountryBalances,
} from './balanceOfPowerData.ts';
import { balanceOfPowerProjection } from './balanceOfPowerProjection.ts';

const run = (firstHour: number, lastHour: number, value: string) => ({ firstHour, lastHour, value });
const balance = (token: string, values: ReturnType<typeof run>[], extra: Partial<BalanceHistory> = {}): BalanceHistory => ({
  token,
  name: null,
  left: { token: `${token}_left`, name: null },
  right: { token: `${token}_right`, name: null },
  ranges: [
    { token: `${token}_low`, name: 'Low', min: '-100000', max: '-25000' },
    { token: `${token}_high`, name: 'High', min: '25000', max: '100000' },
  ],
  values,
  record: values.map(item => run(item.firstHour, item.lastHour, '0')),
  range: values.map(item => run(item.firstHour, item.lastHour, BigInt(item.value) > 25000n ? `${token}_high` : BigInt(item.value) < -25000n ? `${token}_low` : `${token}_mid`)),
  trending: values.map(item => run(item.firstHour, item.lastHour, `${token}_right`)),
  ...extra,
});
const history = (fields: Record<string, unknown>, mode = 'country'): PoliticsHistoryReply => ({
  branchId: '1',
  mode: mode as PoliticsHistoryReply['mode'],
  lastHour: 10_000,
  tag: mode === 'country' ? 'MOD' : null,
  known: [{ firstHour: 0, lastHour: 10_000 }],
  elections: [],
  names: {},
  holderBalances: [],
  warnings: [],
  ...fields,
});
const holder = (id: string, tags: [number, string][], balances: BalanceHistory[], shown: ReturnType<typeof run>[] = []) =>
  ({ holder: id, tags: tags.map(([hour, tag]) => ({ hour, tag })), balances, shown });
const record = (ordinal: number, token: string, members: (string | null)[], extra: Partial<BalanceRecord> = {}): BalanceRecord => ({
  ordinal,
  token,
  name: `${token} name`,
  left: { token: `${token}_left`, name: 'Left' },
  right: { token: `${token}_right`, name: 'Right' },
  trending: { token: `${token}_right`, name: 'Right' },
  range: { token: `${token}_high`, name: 'High', min: '25000', max: '100000' },
  ranges: [],
  members: members.map(tag => ({ tag, daily: '10', weekly: '70' })),
  sources: [{ token: 'mod_source', name: 'Source', daily: '5', weekly: '35' }],
  ...extra,
});
const detail = (balances: BalanceRecord[], extra: Record<string, unknown> = {}): PoliticsStateReply => ({
  branchId: '1',
  view: 'balances',
  exiles: [],
  exilesMissing: null,
  balances,
  current: [{ tag: 'MOD', record: 0 }],
  unavailable: [],
  warnings: [],
  ...extra,
});

const NO_ART: ArtResult = { status: 'noArtExpected' };

test('a country history holds each holder its tag held, and the view follows the one holding it then', () => {
  const data = validateCountryBalances(history({
    holderBalances: [
      holder('1', [[0, 'MOD'], [500, 'NEW']], [balance('guild', [run(0, 900, '30000')])], [run(0, 900, 'guild')]),
      holder('2', [[600, 'MOD']], [balance('court', [run(600, 900, '-5000')])], [run(600, 900, 'court')]),
    ],
  }));
  assert.equal(countryHolder(data, 'MOD', 100)!.holder, '1');
  assert.equal(countryHolder(data, 'MOD', 700)!.holder, '2', 'a reused tag is another holder');
  assert.equal(countryHolder(data, 'MOD', 550)!.holder, '1', 'between holders, the one that held it last');
  assert.throws(() => validateCountryBalances(history({ holderBalances: [holder('1', [[0, 'MOD']], [balance('a', [run(0, 10, '1.5')])])] })));
  assert.throws(() => validateCountryBalances(history({}, 'elections')));
});

test('validators accept optional refusal reasons on sides, ranges, and records, and balance helpers stay exact', () => {
  const guild = balance('guild', [run(0, 99, '30000'), run(200, 400, '-30000')]);
  const court = balance('court', [run(50, 400, '1000')]);
  const subject = { balances: [guild, court], shown: [run(0, 99, 'guild'), run(100, 400, 'court')] };
  assert.equal(chooseBalance(subject, '', 10, 400).balance?.token, 'guild');
  assert.equal(chooseBalance(subject, '', 150, 400).balance?.token, 'court');
  assert.equal(chooseBalance(subject, 'guild', 150, 400).balance?.token, 'guild');
  assert.deepEqual(balanceAbsence([{ firstHour: 0, lastHour: 99 }], 150, 400, [{ firstHour: 100, lastHour: 199 }]), { kind: 'unread' });
  assert.deepEqual(balanceAxis(guild), { min: -1, max: 1, ticks: [-1, -0.25, 0.25, 1] });

  const stateReply = validateBalanceList(detail([
    record(0, 'guild', ['MOD', 'OTH'], {
      left: { token: 'guild_left', name: null, reason: 'no localisation in the recorded content' },
      right: { token: 'guild_right', name: 'Council' },
    }),
  ]));
  assert.equal(recordFor(stateReply, null, 'MOD')?.token, 'guild');
  assert.equal(holderRecord({ ...stateReply, hour: 100 }, holder('1', [[0, 'MOD']], [guild]), 'guild')?.ordinal, 0);
  assert.equal(sumWeeklyDrift(stateReply.balances[0]!), 140n);
});

test('balance-of-power time-chart projection renders pre-first-reading hatched gap, causal side overlays, and hover refusal reasons', () => {
  const hist = history({
    names: { ita_bop: 'Power Balance' },
    holderBalances: [
      holder('1', [[0, 'ITA']], [balance('ita_bop', [run(500, 800, '36000'), run(801, 1200, '50000')])], [run(500, 1200, 'ita_bop')]),
    ],
  });
  const stateAt800 = detail([
    record(0, 'ita_bop', ['ITA'], {
      name: 'Power Balance',
      left: { token: 'ita_left', name: 'Grand Council of Fascism' },
      right: { token: 'ita_right', name: 'Benito Mussolini' },
      range: { token: 'ita_high', name: 'Consolidated Power', min: '25000', max: '100000' },
    }),
  ], { current: [{ tag: 'ITA', record: 0 }] });
  const hoverAt600 = detail([
    record(0, 'ita_bop', ['ITA'], {
      name: 'Power Balance',
      left: { token: 'ita_left', name: null, reason: 'characters table was not recorded' },
      right: { token: 'ita_right', name: 'Benito Mussolini' },
      range: { token: 'ita_high', name: 'Early Power', min: '25000', max: '100000' },
    }),
  ], { current: [{ tag: 'ITA', record: 0 }] });

  const view = createBoundedReadView({
    hour: 800,
    panels: new Map<string, unknown>([['history', hist], ['state', stateAt800]]),
    hovers: new Map([['hover', { hour: 600, data: hoverAt600 }]]),
    art: { resolve: () => NO_ART },
  });
  const model = balanceOfPowerProjection.project(view, {
    tag: 'ITA',
    extent: [0, 1000],
    branchFirstHour: 0,
    branchGaps: [],
    gaps: [],
  });

  assert.equal(model.mode, 'lines');
  assert.deepEqual(model.yAxis.fixed, [-1, 1]);
  assert.equal(model.series.length, 1);
  assert.equal(model.series[0]!.step, true);
  assert.equal(model.series[0]!.headValue, 0.36);
  assert.equal(model.series[0]!.endLabel?.value, '0.36');
  assert.ok(model.series[0]!.points!.every(pt => pt.hour <= 800), 'no future points past playhead');
  assert.equal(model.gaps?.length, 1);
  assert.deepEqual([model.gaps![0]!.firstHour, model.gaps![0]!.lastHour], [0, 500]);
  assert.deepEqual(model.overlays?.map(o => [o.position, o.arrow, o.text]), [
    ['top-right', 'right', 'Benito Mussolini'],
    ['bottom-right', 'left', 'Grand Council of Fascism'],
  ]);
  assert.equal(model.hoverDetail?.hour, 600);
  assert.ok(model.hoverDetail?.items.some(i => i.label === 'Range' && i.value === 'Early Power'));
  assert.ok(model.hoverDetail?.items.some(i => i.label.includes(SIDE_UNAVAILABLE) && i.value === 'characters table was not recorded'));
});
