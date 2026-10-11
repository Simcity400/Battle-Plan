import assert from 'node:assert/strict';
import test from 'node:test';
import { tooltipText } from '../../charts/tooltip.ts';
import { tooltipRowOffsets, tooltipRowWindow } from '../../charts/tooltipScroll.ts';
import { transferNodeTooltip, transferRibbonTooltip, transferTipRequest } from './territoryTooltip.ts';
import type { TransferTipRequest } from './territoryTooltip';
import type { TransferLink } from './territoryData';
import { formatDateTime } from '../../dashboard/calendar.ts';

const states: TransferLink['states'] = [10, 20].map((afterHour, index) => ({ state: 9, dates: { beforeHour: afterHour - (index ? 2 : 0), afterHour },
  fromCountry: { slot: 1, countryId: 101, tag: 'Same tag', hour: afterHour }, toCountry: { slot: 2, countryId: 102, tag: 'Winner', hour: afterHour },
  victoryPoints: index ? null : '11', victoryPointsReadHour: index ? null : 12, from: 'Same tag', to: 'Winner', derived: index > 0 }));
const part: TransferLink = { id: 'part', source: '[1,101]', target: '[2,102]', from: 'Same tag', to: 'Winner', stateCount: 2, victoryPoints: null, states };
const source = { id: 'source', side: 'source' as const, tag: 'Same tag', members: ['Same tag'], memberValues: new Map(), value: 2 };
const target = { ...source, id: 'target', side: 'target' as const, tag: 'Winner' };

test('campaign hovers request the same country separately at each occurrence hour', () => {
  assert.deepEqual(transferTipRequest(states).countries, [
    { hour: 10, tag: 'Same tag' }, { hour: 10, tag: 'Winner' }, { hour: 20, tag: 'Same tag' }, { hour: 20, tag: 'Winner' },
  ]);
  assert.deepEqual(transferTipRequest(states).states, [{ hour: 10, state: 9 }, { hour: 20, state: 9 }]);
});

test('tooltips retain exact and bounded dates, occurrence-hour labels, VP values and unknown totals', () => {
  const tip = transferRibbonTooltip({ id: 'link', source: source.id, target: target.id, value: 2, parts: [part] }, source, target, {
    nodeName: node => node.tag!, labelAt: hour => tag => ({ name: `${tag} at ${hour}`, flag: `flag ${hour}` }),
    stateNameAt: (hour, state) => `State ${state} at ${hour}`, ask: () => {},
  });
  const text = tooltipText(tip);
  assert.match(text, /Same tag at 20/);
  assert.match(text, /Same tag at 10/);
  assert.match(text, /Victory points Not recorded/);
  assert.doesNotMatch(text, /captured by the finalizer/);
  assert.doesNotMatch(text, /derived from what changed/);
  assert.doesNotMatch(text, /Captured/);
  assert.deepEqual(tip.rows.filter(row => row.label.startsWith('State 9')).map(row => row.value),
    [`By ${formatDateTime(20)}`, formatDateTime(10)]);
  assert.doesNotMatch(text, /compared with/);
  assert.ok(tip.rows.some(row => row.value === 'Same tag at 10' && row.countryFlag?.flag === 'flag 10'));
  assert.ok(tip.rows.some(row => row.value === 'Winner at 20' && row.countryFlag?.flag === 'flag 20'));
  assert.ok(tip.rows.some(row => row.value === '11'));
});

test('a scrollable tooltip keeps totals and every occurrence visually reachable', () => {
  const tip = transferRibbonTooltip({ id: 'link', source: source.id, target: target.id, value: 2, parts: [part] }, source, target, {
    nodeName: node => node.tag!, labelAt: () => tag => ({ name: tag }),
    stateNameAt: (_, state) => `State ${state}`, ask: () => {},
  });
  assert.ok(tip.rows.some(row => row.label === 'Transfers' && row.value === '2'));
  assert.equal(tip.scrollable, true);
  assert.equal(tip.rows.filter(row => row.label === 'State 9').length, 2);
  assert.doesNotMatch(tooltipText(tip), /Captured/);
});

test('unknown transfer dates keep their interval without acquisition labels', () => {
  const unknown = { ...part, stateCount: 1, states: [{ ...states[0], derived: null }] };
  const tip = transferRibbonTooltip({ id: 'link', source: source.id, target: target.id, value: 1, parts: [unknown] }, source, target, {
    nodeName: node => node.tag!, labelAt: () => tag => ({ name: tag }),
    stateNameAt: (_, state) => `State ${state}`, ask: () => {},
  });
  assert.doesNotMatch(tooltipText(tip), /Read dates/);
  assert.equal(tip.rows.find(row => row.label === 'State 9')?.value,
    `${formatDateTime(states[0].dates.beforeHour)} – ${formatDateTime(states[0].dates.afterHour)}`);
  assert.doesNotMatch(tooltipText(tip), /Captured/);
});

test('node tooltip titles and flags use the exact latest occurrence label rather than the sampled chart label', () => {
  const tip = transferNodeTooltip(source, [{ id: 'link', source: source.id, target: target.id, value: 2, parts: [part] }], {
    nodeName: () => 'Country name from an older chart sample',
    labelAt: hour => tag => ({ name: `${tag} at ${hour}`, flag: `flag ${hour}` }), stateNameAt: (_, state) => `State ${state}`, ask: () => {},
  });
  assert.equal(tip.title, 'Same tag at 20');
  assert.equal(tip.countryFlag?.flag, 'flag 20');
  assert.doesNotMatch(tooltipText(tip), /older chart sample/);
});

test('native flag refusal and pending metadata keep each occurrence hour', () => {
  const tip = transferNodeTooltip(source, [{ id: 'link', source: source.id, target: target.id, value: 2, parts: [part] }], {
    nodeName: () => 'Older chart label',
    labelAt: hour => tag => ({ name: `${tag} at ${hour}`, flagReason: `Flag unavailable at ${hour}`, flagPending: hour === 10 }),
    stateNameAt: (_, state) => `State ${state}`, ask: () => {},
  });
  assert.deepEqual(tip.countryFlag, { name: 'Same tag at 20', flagReason: 'Flag unavailable at 20', flagPending: false });
  assert.deepEqual(tip.rows.find(row => row.value === 'Same tag at 10')?.countryFlag,
    { name: 'Same tag at 10', flagReason: 'Flag unavailable at 10', flagPending: true });
});

test('opaque empty tags retain their occurrence-hour label and flag', () => {
  const giver = { ...source, tag: '' };
  const empty = { ...part, states: part.states.map(state => ({ ...state, from: '', fromCountry: { ...state.fromCountry, tag: '' } })) };
  const tip = transferRibbonTooltip({ id: 'link', source: source.id, target: target.id, value: 2, parts: [empty] }, giver, target, {
    nodeName: () => 'Older chart label',
    labelAt: hour => tag => ({ name: `${tag || 'Empty tag'} at ${hour}`, flag: `flag ${hour}` }),
    stateNameAt: (_, state) => `State ${state}`, ask: () => {},
  });
  assert.equal(tip.title, 'Empty tag at 20 → Winner at 20');
  assert.equal(tip.countryFlag?.flag, 'flag 20');
});

test('a future or undated VP read stays unavailable in the occurrence tooltip', () => {
  const pending = { ...part, stateCount: 1, states: [{ ...states[0], victoryPoints: null, victoryPointsPending: true }] };
  const options = { nodeName: () => 'Country',
    labelAt: () => (tag: string) => ({ name: tag }), stateNameAt: (_: number, state: number) => `State ${state}`, ask: () => {} };
  const tip = transferNodeTooltip(source, [{ id: 'link', source: source.id, target: target.id, value: 1, parts: [pending] }], options);
  assert.match(tooltipText(tip), /Not available at this time/);
  assert.doesNotMatch(tooltipText(tip), /11; read/);
  pending.states[0].victoryPointsReadHour = null;
  const undated = transferNodeTooltip(source, [{ id: 'link', source: source.id, target: target.id, value: 1, parts: [pending] }], options);
  assert.match(tooltipText(undated), /Not available at this time/);
});

test('campaign tooltip requests and retains all 5,000 visible and spoken occurrences', () => {
  const make = (count: number) => {
    const occurrences = Array.from({ length: count }, (_, index) => ({ ...states[0], state: index,
      fromCountry: { ...states[0].fromCountry, hour: index }, toCountry: { ...states[0].toCountry, hour: index },
      dates: { beforeHour: index, afterHour: index }, victoryPoints: '1', victoryPointsReadHour: index, derived: index % 2 === 0 }));
    const own: TransferLink = { ...part, stateCount: count, victoryPoints: BigInt(count), states: occurrences };
    let asked: TransferTipRequest | null = null;
    const render = () => transferNodeTooltip(source, [{ id: 'link', source: source.id, target: target.id, value: count, parts: [own] }], {
      nodeName: () => 'Country', labelAt: () => tag => ({ name: tag }), stateNameAt: (_, state) => `State ${state}`,
      ask: request => { asked = request; },
    });
    return { render, request: () => asked };
  };
  const large = make(5_000);
  const tip = large.render();
  assert.equal(large.request()!.states.length, 5_000);
  assert.equal(large.request()!.countries.length, 10_000);
  assert.equal(large.request()!.states[0].state, 4_999);
  assert.equal(large.request()!.states[4_999].state, 0);
  assert.ok(tip.rows.some(row => row.label === 'Transfers' && row.value === '5,000'));
  assert.doesNotMatch(tooltipText(tip), /captured by the finalizer/);
  assert.doesNotMatch(tooltipText(tip), /derived from what changed/);
  assert.match(tooltipText(tip), /State 4999/);
  assert.match(tooltipText(tip), /State 0 By/);
  assert.equal(tip.rows.filter(row => row.label.startsWith('State ')).length, 5_000);
  assert.equal(tip.scrollable, true);
});

test('a summed other node retains all country facts and occurrence rows beyond the former eight-row hover limit', () => {
  const occurrences = Array.from({ length: 40 }, (_, state) => ({ ...states[0], state, from: `Giver ${state}`,
    fromCountry: { ...states[0].fromCountry, tag: `Giver ${state}`, hour: state }, toCountry: { ...states[0].toCountry, hour: state },
    dates: { beforeHour: state, afterHour: state }, victoryPoints: '1', victoryPointsReadHour: state }));
  const own: TransferLink = { ...part, stateCount: occurrences.length, victoryPoints: 40n, states: occurrences };
  const other = { ...source, tag: null, members: occurrences.map(state => state.from) };
  let asked: TransferTipRequest | null = null;
  const tip = transferNodeTooltip(other, [{ id: 'link', source: other.id, target: target.id, value: 40, parts: [own] }], {
    nodeName: () => '40 other givers', labelAt: hour => tag => ({ name: `${tag} at ${hour}`, flag: `flag ${hour}` }),
    stateNameAt: (_, state) => `State ${state}`, ask: request => { asked = request; },
  });
  assert.equal(asked!.countries.length, 80);
  assert.equal(asked!.states.length, 40);
  assert.equal(tip.countryFlag, undefined, 'aggregate nodes have no native country flag');
  for (let state = 0; state < 40; state++) assert.ok(tooltipText(tip).includes(`Giver ${state} at ${state}`));
  assert.equal(tip.scrollable, true, 'complete rows remain visually reachable as well as spoken');
});

test('a taller tooltip shows more than eight occurrences solely according to viewport room', () => {
  const occurrences = Array.from({ length: 20 }, (_, state) => ({ ...states[0], state,
    dates: { beforeHour: state, afterHour: state }, victoryPoints: '1', victoryPointsReadHour: state }));
  const own: TransferLink = { ...part, stateCount: occurrences.length, victoryPoints: 20n, states: occurrences };
  const tip = transferNodeTooltip(source, [{ id: 'link', source: source.id, target: target.id, value: 20, parts: [own] }], {
    nodeName: () => 'Country', labelAt: () => tag => ({ name: tag }),
    stateNameAt: (_, state) => `State ${state}`, ask: () => {},
  });
  const offsets = tooltipRowOffsets(tip.rows);
  const wide = tooltipRowWindow(offsets, 0, 8_000);
  const narrow = tooltipRowWindow(offsets, 0, 500);
  assert.equal(tip.rows.slice(wide.first, wide.last).filter(row => row.label.startsWith('State ')).length, 20);
  assert.ok(tip.rows.slice(narrow.first, narrow.last).filter(row => row.label.startsWith('State ')).length < 20);
  const end = tooltipRowWindow(offsets, Infinity, 500);
  assert.ok(tip.rows.slice(end.first, end.last).some(row => row.label === 'State 0'), 'scrolling reaches the entire remainder');
  for (let state = 0; state < 20; state++) assert.equal(tip.rows.find(row => row.label === `State ${state}`)?.value, formatDateTime(state));
});


test('giver and taker labels use their own supplying dates through rename and tag reuse', () => {
  const earlier = { ...states[0], fromCountry: { slot: 1, countryId: 101, tag: 'X', hour: 9 }, toCountry: { slot: 2, countryId: 102, tag: 'Y', hour: 10 } };
  const occurrence = { ...part, stateCount: 1, states: [earlier] };
  assert.deepEqual(transferTipRequest([earlier]).countries, [{ tag: 'X', hour: 9 }, { tag: 'Y', hour: 10 }]);
  const tip = transferRibbonTooltip({ id: 'native', source: source.id, target: target.id, value: 1, parts: [occurrence] }, source, target, {
    nodeName: () => 'Latest replacement', labelAt: hour => tag => ({ name: `${tag} holder at ${hour}` }), stateNameAt: () => 'State', ask() {},
  });
  assert.equal(tip.title, 'X holder at 9 \u2192 Y holder at 10');
  assert.doesNotMatch(tooltipText(tip), /Latest replacement/);
});
