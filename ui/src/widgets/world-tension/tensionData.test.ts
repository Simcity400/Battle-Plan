import test from 'node:test';
import assert from 'node:assert/strict';
import { stackedSegments } from '../../charts/stack.ts';
import { groupMembers, groupRuns, worldFilter } from '../../dashboard/groups.ts';
import { Lineage } from '../../dashboard/identity.ts';
import { countryIdentityIndex, countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import type { CountryGroupReply, TensionReply } from '../../generated/dashboard';
import { addedAt, contributionAt, countryKey, eventGroup, eventsOutsideReads, eventsWithin, formatContribution, formatTension, formatTensionAxis, layerKeys, otherRaw,
  layerTagAt, rankedAt, readAt, shareRaw, tensionModel, tensionPlot, tensionStack, validateTensionReply } from './tensionData.ts';

const run = (firstHour: number, lastHour: number, valueRaw: string, startsSegment = false) => ({ firstHour, lastHour, valueRaw, startsSegment });
const read = (firstHour: number, lastHour: number, tensionRaw: string, sumRaw: string, startsSegment = false, unnamedRaw = '0') =>
  ({ firstHour, lastHour, startsSegment, tensionRaw, sumRaw, unnamedRaw });
// Each country's canonical slot in the fixture, and its layer key.
const SLOTS: Record<string, number> = { 'D01 mod': 31, GER: 7, ITA: 8, CZE: 12 };
const key = (tag: string) => countryKey({ tag, slot: SLOTS[tag] });

// World tension in thousandths of a percent, contributions in hundred-thousandths.
// Day 1: 50% from three countries (a modded dynamic tag, spaces and all, among
// them). Day 2: the sources pass the cap (133.5%) and one is a peace treaty (-2%).
// Then a gap, and a day read afresh.
const REPLY: TensionReply = {
  branchId: '1', tensionScale: 1000, contributionScale: 100_000, ceilingRaw: '10000000',
  reads: [read(100, 123, '50000', '5000000', true), read(124, 147, '100000', '13350000', false, '50000'), read(200, 223, '3000', '300000', true)],
  countries: [
    { tag: 'D01 mod', slot: 31, runs: [run(100, 147, '1000000', true)] },
    { tag: 'GER', slot: 7, runs: [run(100, 123, '3000000', true), run(124, 147, '10000000'), run(200, 223, '300000', true)] },
    { tag: 'ITA', slot: 8, runs: [run(100, 123, '1000000', true), run(124, 147, '2500000')] },
    { tag: 'CZE', slot: 12, runs: [run(124, 147, '-200000')] },
  ],
  events: [
    { hour: 90, tag: 'GER', reason: 'Remilitarised the Rhineland', added: [{ hour: 100, addedRaw: '3000000' }], firstRead: 100, lastRead: 147 },
    { hour: 130, tag: 'ITA', reason: 'Italian Irredentism', added: [{ hour: 147, addedRaw: '1500000' }, { hour: 200, addedRaw: '0' }], firstRead: 147, lastRead: 200 },
    { hour: 131, tag: null, reason: 'Threat from actions', added: [{ hour: 147, addedRaw: '50000' }], firstRead: 147, lastRead: 147 },
  ],
  warnings: [],
};

test('country filters and membership retain renamed contributors and distinguish a reused tag by slot', () => {
  const lineage = new Lineage([{ hour: 120, from: 'AAA', to: 'BBB' }]);
  const countries = [
    { tag: 'AAA', slot: 1, runs: [run(100, 119, '1000000', true)], identities: [{ firstHour: 100, lastHour: 119, slot: 1, countryId: 10 }] },
    { tag: 'BBB', slot: 1, runs: [run(120, 147, '1000000', true)], identities: [{ firstHour: 120, lastHour: 147, slot: 1, countryId: 10 }] },
    { tag: 'AAA', slot: 2, runs: [run(124, 147, '2000000', true)], identities: [{ firstHour: 124, lastHour: 147, slot: 2, countryId: 20 }] },
  ];
  const metadata = [{ id: '1', tag: 'AAA', name: 'Original', observedBranchId: '1', observedHour: 100 },
    { id: '2', tag: 'AAA', name: 'Later holder', observedBranchId: '1', observedHour: 124 }];
  const model = tensionModel({ ...REPLY, countries }, lineage, metadata);
  const identities = countryIdentityIndex([{ tag: 'AAA', identities: [...countries[0].identities, ...countries[2].identities] }, { tag: 'BBB', identities: countries[1].identities }]);
  const filter = worldFilter({ kind: 'tags', tags: ['BBB'] }, [], undefined, 130, 147, identities);
  assert.deepEqual(layerKeys(model, 130, 5, filter.tags, filter.countryIds), [countryKey(countries[1])]);
  assert.equal(rankedAt(model, 130, filter.tags, filter.countryIds)[0].tag, 'BBB');
  const membership = new Map([[countryIdentityKey(countries[0].identities[0]), [{ firstHour: 100, lastHour: 130 }]]]);
  const stack = tensionStack(model, [countryKey(countries[1])], false, membership, true, filter.countryIds);
  const before = stack.hours.findIndex(hour => hour === 124);
  const after = stack.hours.findIndex(hour => hour === 131);
  assert.equal(stack.edges[before]?.[0].high, 10);
  assert.equal(stack.edges[after]?.[0].high, 0);
  const many = tensionModel({ ...REPLY, countries: Array.from({ length: 5000 }, (_, index) => ({ tag: `mod ${index}`, slot: index, runs: [run(100, 147, '1')] })) }, lineage);
  assert.equal(rankedAt(many, 130).length, 5000);
});

test('the reply is checked, and malformed values are refused', () => {
  assert.equal(validateTensionReply(REPLY), REPLY);
  assert.throws(() => validateTensionReply({ ...REPLY, tensionScale: 1500 }));
  assert.throws(() => validateTensionReply({ ...REPLY, reads: [REPLY.reads[1], REPLY.reads[0]] }));
  assert.throws(() => validateTensionReply({ ...REPLY, countries: [{ tag: 'GER', slot: 7, runs: [run(1, 2, '1.5')] }] }));
  // A country without its identity, or listed twice, is refused.
  assert.throws(() => validateTensionReply({ ...REPLY, countries: [{ tag: 'GER', runs: [] } as never] }));
  assert.throws(() => validateTensionReply({ ...REPLY, countries: [REPLY.countries[1], REPLY.countries[1]] }));
  assert.throws(() => validateTensionReply({ ...REPLY, events: [{ ...REPLY.events[0], added: [] }] }));
});

test('a read holds its values to its last hour; a country without a run there contributed zero, and a gap is unknown', () => {
  const model = tensionModel(REPLY);
  assert.equal(readAt(model, 130)?.firstHour, 124);
  assert.equal(readAt(model, 170), null);
  assert.equal(contributionAt(model, key('CZE'), 110), '0');
  assert.equal(contributionAt(model, key('CZE'), 130), '-200000');
  assert.equal(contributionAt(model, key('D01 mod'), 170), null);
  assert.equal(formatTension(model, '100000'), '100.0%');
  assert.equal(formatContribution(model, '-200000', true), '−2.00%');
  assert.equal(formatContribution(model, '1000000', true), '+10.00%');
});

test('layers are the largest contributors at the moment shown, in rank order, with the rest as Other', () => {
  const model = tensionModel(REPLY);
  assert.deepEqual(rankedAt(model, 110).map(item => item.tag), ['GER', 'D01 mod', 'ITA']);
  assert.deepEqual(rankedAt(model, 130).map(item => item.tag), ['GER', 'ITA', 'D01 mod', 'CZE']);
  assert.deepEqual(layerKeys(model, 130, 2), [key('GER'), key('ITA')]);
  assert.deepEqual(rankedAt(model, 170), []);
  // Other is the sum less the named layers, so it keeps what no recorded country names (0.5%) and the treaty.
  assert.equal(otherRaw(model, REPLY.reads[1], [key('GER'), key('ITA')]), '850000');
});

test('shares scale every contribution so they add up to world tension, which the game caps', () => {
  const model = tensionModel(REPLY);
  const capped = REPLY.reads[1];
  const shares = ['GER', 'ITA', 'D01 mod', 'CZE'].map(tag => BigInt(shareRaw(model, capped, contributionAt(model, key(tag), 130)!)))
    .concat(BigInt(shareRaw(model, capped, capped.unnamedRaw!)));
  const total = shares.reduce((sum, value) => sum + value, 0n);
  // 100% in contribution units, less rounding toward zero.
  assert.ok(total <= 10_000_000n && total > 10_000_000n - 5n, String(total));
  // Below the cap a share is the contribution itself.
  assert.equal(shareRaw(model, REPLY.reads[0], '3000000'), '3000000');
  // A sum that is not positive leaves world tension at zero, and every share with it.
  assert.equal(shareRaw(model, read(1, 1, '0', '-5'), '100'), '0');
});

test('the stack follows the reads, breaks at the gap and adds Other on top', () => {
  const model = tensionModel(REPLY);
  const recorded = stackedSegments(tensionStack(model, [key('GER'), key('ITA')], false), 100, 223, 0);
  // Each read held to its last hour, the next read from the hour after; the gap after 147 breaks it.
  assert.deepEqual(recorded.map(segment => segment.hours), [[100, 123, 124, 147, 148], [200, 223, 224]].map(hours => hours.slice(0, -1)));
  // Layers stack in order: GER, then ITA on it, then Other (the sum less both) on top.
  assert.deepEqual(recorded[0].bands.map(band => band.high), [[30, 30, 100, 100], [40, 40, 125, 125], [50, 50, 133.5, 133.5]]);
  const shares = stackedSegments(tensionStack(model, [key('GER'), key('ITA')], true), 124, 147, 0)[0];
  const top = shares.bands.at(-1)!.high.at(-1)!;
  assert.ok(Math.abs(top - 100) < 1e-3, String(top));
  // A treaty's negative contribution stacks below zero in Other when it is not a layer of its own.
  const signed = stackedSegments(tensionStack(model, [key('CZE')], false), 124, 147, 0)[0];
  assert.deepEqual(signed.bands[0].low, [-2, -2]);
});

test('hourly membership keeps a later top contributor in Other until it joins, including between daily reads', () => {
  const reply: TensionReply = { ...REPLY, events: [],
    reads: [read(100, 147, '60000', '6000000', true), read(196, 219, '60000', '6000000', true), read(220, 300, '60000', '6000000')],
    countries: ['AAA', 'BBB', 'CCC'].map((tag, slot) => ({ tag, slot, runs: [run(100, 147, `${3_000_000 - slot * 1_000_000}`, true),
      run(196, 300, `${3_000_000 - slot * 1_000_000}`, true)] })) };
  const groups: CountryGroupReply = { branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], factions: [], wars: [],
    countryIdentities: reply.countries.map(country => ({ tag: country.tag, identities: [{ firstHour: 100, lastHour: 300, slot: country.slot, countryId: country.slot }] })),
    unobserved: { humans: [], factions: [], wars: [] },
    humans: [{ tag: 'CCC', firstHour: 100, lastHour: 300 }, { tag: 'AAA', firstHour: 200, lastHour: 249 }, { tag: 'AAA', firstHour: 280, lastHour: 300 }] };
  const model = tensionModel(reply);
  const runs = groupRuns(groups, { type: 'humans' })!;
  const members = (hour: number) => new Set(groupMembers(runs, 'hour', hour, 300)!);
  const keys = layerKeys(model, 300, 1, members(300));
  assert.deepEqual(keys, [countryKey(reply.countries[0])]);
  const membership = new Map(['AAA', 'CCC'].map(tag => [tag, runs.filter(run => run.tag === tag)]));
  for (const share of [true, false]) {
    const stack = tensionStack(model, keys, share, membership);
    for (const [hour, named] of [[110, 0], [199, 0], [200, 30], [249, 30], [250, 0], [279, 0], [280, 30], [300, 30]]) {
      const [segment] = stackedSegments(stack, hour, hour, 0);
      assert.equal(segment.bands[0].high[0], named, `named at ${hour}`);
      assert.equal(segment.bands[1].high[0] - segment.bands[1].low[0], 60 - named, `Other at ${hour}`);
    }
    assert.deepEqual(stackedSegments(stack, 170, 180, 0), []);
  }
  // The tooltip ranks the hovered hour's members, not the countries chosen at the end.
  const selection = { kind: 'group' as const, group: { type: 'humans' as const }, membership: 'hour' as const, aggregate: false };
  assert.deepEqual(rankedAt(model, 199, worldFilter(selection, [], groups, 199, 300).tags).map(item => item.tag), ['CCC']);
  assert.deepEqual(rankedAt(model, 200, worldFilter(selection, [], groups, 200, 300).tags).map(item => item.tag), ['AAA', 'CCC']);
  // The other membership modes still show the chosen country's whole history.
  for (const mode of ['end', 'ever'] as const) {
    const keys = layerKeys(model, 300, 1, worldFilter({ ...selection, membership: mode }, [], groups, 300, 300).tags);
    assert.equal(stackedSegments(tensionStack(model, keys, true), 110, 110, 0)[0].bands[0].high[0], 30);
  }
});

test('native selection keeps both X holders, excludes unrelated Y, and clips history at missing native spans', () => {
  const countries = [
    { tag: 'X', slot: 1, runs: [run(0, 30, '100000')], identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 30, slot: 1, countryId: 20 }] },
    { tag: 'Y', slot: 1, runs: [run(10, 19, '200000')], identities: [{ firstHour: 10, lastHour: 19, slot: 1, countryId: 10 }] },
    { tag: 'Y', slot: 2, runs: [run(20, 30, '300000')], identities: [{ firstHour: 20, lastHour: 30, slot: 2, countryId: 30 }] },
  ];
  const model = tensionModel({ ...REPLY, events: [], countries, reads: [read(0, 30, '4000', '400000')] });
  const identities = countryIdentityIndex([{ tag: 'X', identities: countries[0].identities }, { tag: 'Y', identities: [...countries[1].identities, ...countries[2].identities] }]);
  const selected = identities.selected(['X']);
  assert.deepEqual(rankedAt(model, 15, null, selected).map(row => row.key), [countryKey(countries[1])]);
  assert.deepEqual(rankedAt(model, 25, null, selected).map(row => row.key), [countryKey(countries[0])]);
  const stack = tensionStack(model, [countryKey(countries[0]), countryKey(countries[1])], false, null, false, selected);
  const at = (hour: number) => stackedSegments(stack, hour, hour, 0)[0].bands.map(band => band.high[0] - band.low[0]);
  assert.deepEqual(at(5), [1, 0, 3]);
  assert.deepEqual(at(15), [0, 2, 2]);
  assert.deepEqual(at(25), [1, 0, 3]);
});

test('a native contributor alias after a gap labels its own raw layer without a rename event', () => {
  const countries = [
    { tag: 'X', slot: 1, runs: [run(0, 19, '100000')], identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }] },
    { tag: 'Y', slot: 1, runs: [run(11, 19, '200000')], identities: [{ firstHour: 11, lastHour: 19, slot: 1, countryId: 10 }] },
  ];
  const model = tensionModel({ ...REPLY, events: [], countries, reads: [read(0, 10, '1000', '100000'), read(11, 19, '3000', '300000')] }, new Lineage([]));
  const identities = countryIdentityIndex(countries.map(country => ({ tag: country.tag, identities: country.identities })));
  const [x, y] = countries.map(countryKey);
  assert.deepEqual(rankedAt(model, 15, null, identities.selected(['X'])), [{ key: y, tag: 'Y', raw: '200000' }]);
  assert.equal(layerTagAt(model, x, 10), null);
  assert.equal(layerTagAt(model, y, 10), null);
  // Stale native contributions are retained even when their current holder cannot be named.
  assert.deepEqual(rankedAt(model, 15), [{ key: y, tag: 'Y', raw: '200000' }, { key: x, tag: null, raw: '100000' }]);
  assert.deepEqual(rankedAt(model, 10), [{ key: x, tag: null, raw: '100000' }]);
  assert.equal(model.raw.get(x), countries[0].runs);
  assert.equal(model.raw.get(y), countries[1].runs);
  assert.equal(contributionAt(model, x, 15), '100000');
  assert.equal(contributionAt(model, y, 15), '200000');
  assert.equal(readAt(model, 15)?.firstHour, 11, 'Y is supplied by a fresh read, not inside a prior held read');
  const keys = [x, y];
  assert.equal(otherRaw(model, readAt(model, 15)!, keys), '0');
  const [segment] = stackedSegments(tensionStack(model, keys, false), 15, 15, 0);
  assert.deepEqual(segment.bands.map((band, index) => ({ key: keys[index] ?? 'Other', value: band.high[0] - band.low[0] })),
    [{ key: x, value: 1 }, { key: y, value: 2 }, { key: 'Other', value: 0 }]);
});

test('a later holder on one canonical slot keeps its recorded alias and an explicit missing suffix refuses a label', () => {
  const countries = [
    { tag: 'OLD', slot: 1, runs: [run(0, 9, '100000')], identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }] },
    { tag: 'NEW', slot: 1, runs: [run(10, 19, '200000')], identities: [{ firstHour: 10, lastHour: 19, slot: 1, countryId: 20 }] },
    { tag: 'UNKNOWN', slot: 2, runs: [run(10, 19, '50000')], identities: [] },
  ];
  const model = tensionModel({ ...REPLY, events: [], countries, reads: [read(0, 19, '2500', '250000')] }, new Lineage([]),
    [{ id: '1', tag: 'OLD', name: 'Earliest', observedBranchId: '1', observedHour: 0 }]);
  const ranked = rankedAt(model, 15);
  assert.deepEqual(ranked.map(row => [row.tag, row.raw]), [['NEW', '200000'], [null, '50000']]);
  assert.deepEqual(rankedAt(model, 15, null, new Set(['[1,10]'])), []);
  assert.deepEqual(rankedAt(model, 15, null, new Set(['[1,20]'])).map(row => row.tag), ['NEW']);
  assert.equal(layerTagAt(model, countryKey(countries[2]), 15), null);
});

test('world tension uses exactly the stack samples through reads, membership boundaries, zoom and gaps', () => {
  const model = tensionModel({ ...REPLY, reads: [read(100, 123, '50000', '5000000', true), read(124, 147, '100000', '10000000'), read(200, 200, '50000', '5000000', true)],
    countries: [{ tag: 'AAA', slot: 1, runs: [run(100, 123, '5000000', true), run(124, 147, '10000000'), run(200, 200, '5000000', true)] }], events: [] });
  const keys = [countryKey(model.reply.countries[0])];
  const membership = new Map([['AAA', [{ firstHour: 118, lastHour: 130 }]]]);
  for (const [from, to, minHours] of [[100, 223, 0.5], [110, 140, 0.5], [118, 118, 0.5], [100, 223, 30], [200.5, 210, 0.5]]) {
    const segments = stackedSegments(tensionPlot(model, keys, true, membership), from, to, minHours);
    for (const segment of segments) assert.deepEqual(segment.bands.at(-1)!.high, segment.bands.at(-2)!.high);
    assert.ok(segments.every(segment => !(segment.hours[0] < 148 && segment.hours.at(-1)! > 200)));
  }
  const [segment] = stackedSegments(tensionPlot(model, keys, true, membership), 100, 147, 0.5);
  assert.equal(segment.bands.at(-1)!.high[segment.hours.indexOf(118)], 50);
  // The total remains its own recorded value in recorded-contribution mode.
  const [recorded] = stackedSegments(tensionPlot(tensionModel(REPLY), [key('GER')], false), 124, 147, 0.5);
  assert.equal(recorded.bands.at(-1)!.high[0], 100);
  assert.equal(recorded.bands.at(-2)!.high[0], 133.5);
});

test('5,000 countries with 100,000 runs reuse the raw histories for ranking and stacking', () => {
  const countries = Array.from({ length: 5_000 }, (_, slot) => ({ tag: `country ${slot}`, slot,
    runs: Array.from({ length: 20 }, (_, day) => run(day * 24, day * 24 + 23, `${slot + 1}`, day === 0)) }));
  const sumRaw = `${5_000 * 5_001 / 2}`;
  const reply: TensionReply = { ...REPLY, countries, events: [], reads: Array.from({ length: 20 }, (_, day) => read(day * 24, day * 24 + 23, '100000', sumRaw, day === 0)) };
  const model = tensionModel(reply);
  assert.equal('runs' in model, false);
  countries.forEach(country => assert.equal(model.raw.get(countryKey(country)), country.runs));
  assert.deepEqual(layerKeys(model, 479, 2), countries.slice(-2).reverse().map(countryKey));
  const [segment] = stackedSegments(tensionStack(model, layerKeys(model, 479, 8), false), 0, 479, 0.5);
  assert.equal(segment.bands.at(-1)!.high[0], Number(sumRaw) / reply.contributionScale);
});

test('chosen countries narrow the layers to them; everyone else is Other', () => {
  const model = tensionModel(REPLY);
  assert.deepEqual(layerKeys(model, 130, 5, new Set(['ITA', 'CZE', 'D01 mod'])), [key('ITA'), key('D01 mod'), key('CZE')]);
  assert.deepEqual(layerKeys(model, 130, 1, new Set(['CZE', 'SOV'])), [key('CZE')]);
  assert.deepEqual(layerKeys(model, 130, 5, new Set()), []);
});

test('two countries sharing a tag at one read are two layers, never one joined band', () => {
  // Slot 7 held AAA and still contributes after leaving the countries; slot 8 now holds AAA too.
  const reply: TensionReply = { ...REPLY, reads: [read(100, 123, '1000', '100000', true), read(124, 147, '1500', '150000')], events: [],
    countries: [{ tag: 'AAA', slot: 7, runs: [run(100, 147, '100000', true)] }, { tag: 'AAA', slot: 8, runs: [run(124, 147, '50000')] }] };
  const model = tensionModel(validateTensionReply(reply));
  const [older, newer] = reply.countries.map(countryKey);
  assert.deepEqual(rankedAt(model, 130), [{ key: older, tag: 'AAA', raw: '100000' }, { key: newer, tag: 'AAA', raw: '50000' }]);
  // Chosen by tag, both are chosen; each is its own layer and nothing is left in Other.
  assert.deepEqual(layerKeys(model, 130, 5, new Set(['AAA'])), [older, newer]);
  assert.equal(otherRaw(model, reply.reads[1], [older, newer]), '0');
  const [segment] = stackedSegments(tensionStack(model, [older, newer], false), 124, 147, 0);
  assert.deepEqual(segment.bands.map(band => band.high), [[1, 1], [1.5, 1.5], [1.5, 1.5]]);
});

test('the stack breaks where a layer’s own run starts a segment, even inside a read’s line', () => {
  // AAA is one country through 123 and another (its run starts a segment) from 124; the read at 124 does not break.
  const reply: TensionReply = { ...REPLY, reads: [read(100, 123, '1000', '100000', true), read(124, 147, '1000', '100000')], events: [],
    countries: [{ tag: 'AAA', slot: 7, runs: [run(100, 123, '100000', true), run(124, 147, '100000', true)] }] };
  const model = tensionModel(reply);
  const stack = tensionStack(model, [countryKey(reply.countries[0])], false);
  assert.deepEqual(stackedSegments(stack, 100, 147, 0).map(segment => [segment.hours[0], segment.hours.at(-1)]), [[100, 123], [124, 147]]);
  // Without the run's own break the read's line holds, as one band.
  const joined = tensionModel({ ...reply, countries: [{ ...reply.countries[0], runs: [run(100, 123, '100000', true), run(124, 147, '100000')] }] });
  assert.equal(stackedSegments(tensionStack(joined, [countryKey(reply.countries[0])], false), 100, 147, 0).length, 1);
});

test('several events at one hour sum what they generated and list the largest first, either sign', () => {
  const at = (addedRaw: string, reason: string) => ({ hour: 1, tag: 'GER', reason, added: [{ hour: 1, addedRaw }], firstRead: 1, lastRead: 1 });
  const group = eventGroup([at('100', 'a'), at('-300', 'b'), at('200', 'c'), at('100', 'd')]);
  assert.equal(group.totalRaw, '100');
  assert.deepEqual(group.largest.map(event => event.reason), ['b', 'c', 'a', 'd']);
});

test('an event is marked inside the hours shown and adds what was last recorded by then', () => {
  const [, irredentism] = REPLY.events;
  assert.equal(addedAt(irredentism, 120), '1500000');
  assert.equal(addedAt(irredentism, 160), '1500000');
  assert.equal(addedAt(irredentism, 210), '0');
  assert.deepEqual(eventsWithin(REPLY.events, 100, 223).map(event => event.reason), ['Italian Irredentism', 'Threat from actions']);
});

test('value-axis labels take the decimals their ticks need, so ticks zoomed in close never read the same', () => {
  const zoomed = [12.31, 12.32, 12.33, 12.34];
  assert.deepEqual(zoomed.map(tick => formatTensionAxis(tick, zoomed)), ['12.31%', '12.32%', '12.33%', '12.34%']);
  const whole = [0, 25, 50, 75, 100];
  assert.deepEqual(whole.map(tick => formatTensionAxis(tick, whole)), ['0%', '25%', '50%', '75%', '100%']);
  // A negative part (a peace treaty's) is written with a true minus sign.
  const halves = [-2.5, 0, 2.5, 5];
  assert.deepEqual(halves.map(tick => formatTensionAxis(tick, halves)), ['−2.5%', '0.0%', '2.5%', '5.0%']);
});

test('a keyboard stop at an event dated where no read holds the hour shows the event, as its marker does', () => {
  const model = tensionModel(REPLY);
  // Dated inside the gap between the reads (147 and 200), first recorded at the read of 200.
  const gap = { hour: 150, tag: 'GER', reason: 'Declared war', added: [{ hour: 200, addedRaw: '100000' }], firstRead: 200, lastRead: 223 };
  const read = { ...gap, hour: 124, reason: 'Claims' };
  const events = [read, gap];
  assert.equal(readAt(model, 150), null);
  assert.deepEqual(eventsOutsideReads(model, events, 150), [gap]);
  assert.deepEqual(eventsOutsideReads(model, events, 150.5), [gap]);
  // A read's own hour lists its events itself, and an empty gap hour has none.
  assert.deepEqual(eventsOutsideReads(model, events, 124), []);
  assert.deepEqual(eventsOutsideReads(model, events, 151), []);
});
