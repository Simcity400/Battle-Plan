import assert from 'node:assert/strict';
import test from 'node:test';
import { createWidget } from '../../dashboard/model.ts';
import { rankedCandidates, resolveCountries } from '../../dashboard/groups.ts';
import type { CountryFactoriesReply, FactoryField, FactoryGroupKey, FactoryPart } from '../../generated/dashboard';
import { lookPlan } from '../../dashboard/countryLooks.ts';
import { lineLabelHours } from '../../charts/series.ts';
import { DATE_OVERLAY_OPTION, mixCountry } from '../../charts/race/options.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import '../definitions.ts';
import { definition, OPTIONS } from './definition.ts';
import { chosenGroups, countRuns, factoriesAt, factoriesRequest, factoryModel, factorySegmentAt, factoryTooltips, formatCount, groupCount, groupName, groupRows, lineRuns, mixBands, raceEntries, rankFactories, typeChoices, validateFactoriesReply, withheld } from './factoriesData.ts';
import type { CountMode } from './factoriesData';

const GROUP_PARTS: FactoryPart[] = ['total', 'controlled', 'owned', 'damaged', 'fromSubjects', 'toOverlord', 'fromOthers', 'toOthers'];
const CIVILIAN: FactoryPart[] = ['tradeExports', 'licenseExports', 'consumerGoods', 'contractPayments', 'specialProjects', 'licenseImports', 'tradeImports'];
const GROUPS: FactoryGroupKey[] = ['military', 'naval', 'civilian'];
const FIELDS: FactoryField[] = [...GROUPS.flatMap(group => GROUP_PARTS.map(part => ({ group, part }))), ...CIVILIAN.map(part => ({ group: 'civilian' as const, part }))];

/** Values from named parts; every other field is zero. */
function values(parts: Partial<Record<`${FactoryGroupKey}:${FactoryPart}`, number>>): number[] {
  return FIELDS.map(field => parts[`${field.group}:${field.part}`] ?? 0);
}

const GERMANY = values({
  'civilian:total': 40, 'civilian:controlled': 38, 'civilian:owned': 30, 'civilian:damaged': 3,
  'civilian:consumerGoods': 5, 'civilian:tradeImports': 4, 'civilian:specialProjects': 2, 'civilian:licenseImports': 1, 'civilian:contractPayments': 1,
  'civilian:toOverlord': 0, 'civilian:toOthers': 2, 'civilian:tradeExports': 3,
  'military:total': 30, 'military:controlled': 30, 'military:owned': 25, 'military:damaged': 4, 'military:toOverlord': 1, 'military:toOthers': 1,
  'naval:total': 8, 'naval:damaged': 1,
});

const REPLY: CountryFactoriesReply = {
  branchId: '1', warnings: [], fields: FIELDS,
  groups: [
    { key: 'military', flag: 'military_production', buildings: [{ token: 'arms_factory', name: 'Military Factory' }] },
    { key: 'naval', flag: 'naval_production', buildings: [] },
    { key: 'civilian', flag: 'general_production', buildings: [{ token: 'industrial_complex', name: 'Civilian Factory' }, { token: 'mod_works', name: null }] },
  ],
  series: [
    { tag: 'GER', membership: null, segments: [{ firstHour: 100, lastHour: 103 }, { firstHour: 105, lastHour: 106 }, { firstHour: 107, lastHour: 107 }], runs: [
      { firstHour: 100, lastHour: 101, startsSegment: true, values: GERMANY },
      { firstHour: 102, lastHour: 103, startsSegment: false, values: values({ 'civilian:total': 41 }) },
      { firstHour: 105, lastHour: 106, startsSegment: true, values: values({ 'civilian:total': 41 }) },
      { firstHour: 107, lastHour: 107, startsSegment: true, values: values({ 'civilian:total': 41 }) },
    ] },
    { tag: 'ITA', membership: null, segments: [{ firstHour: 100, lastHour: 107 }], runs: [{ firstHour: 100, lastHour: 107, startsSegment: true, values: values({ 'civilian:total': 12, 'military:total': 9 }) }] },
  ],
};

test('factory selection and ranking accept every recorded country', () => {
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const field = OPTIONS.find(field => field.key === 'types')!;
  assert.ok(field.kind === 'content-set');
  const tags = Array.from({ length: 5000 }, (_, index) => `mod country ${index}`);
  const chosen = resolveCountries({ kind: 'tags', tags }, [], undefined, 200, definition.countries.max);
  assert.deepEqual(factoriesRequest('1', chosen, 'built', []), { branchId: '1', tags, top: null });
  assert.equal(chosen.dropped, 0);
  const top = resolveCountries({ kind: 'group', group: { type: 'top', count: tags.length }, membership: 'hour', aggregate: false },
    [], undefined, 200, definition.countries.max);
  assert.equal(top.top, tags.length);
});

test('the reply is checked: one value per field, whole numbers, runs in hour order', () => {
  assert.equal(validateFactoriesReply(REPLY), REPLY);
  const broken = structuredClone(REPLY);
  broken.series[0].runs[1].values = [1];
  assert.throws(() => validateFactoriesReply(broken));
  const overlapping = structuredClone(REPLY);
  overlapping.series[0].runs[1].firstHour = 101;
  assert.throws(() => validateFactoriesReply(overlapping));
  assert.throws(() => validateFactoriesReply(null));
});

test('free after withheld, built and the top bar count as decided (29 Sep)', () => {
  const model = factoryModel(REPLY);
  const count = (group: FactoryGroupKey, mode: CountMode) => groupCount(model, GERMANY, group, mode).count;
  // Civilian: built − damaged − consumer goods − trade − projects − licences − contracts − transfers out.
  assert.equal(withheld(model, GERMANY, 'civilian'), 5 + 4 + 2 + 1 + 1 + 0 + 2);
  assert.equal(count('civilian', 'free'), 40 - 3 - 15);
  assert.equal(count('civilian', 'built'), 40);
  assert.equal(count('civilian', 'topBar'), 40 - 3 - 2);
  // Military and naval: built − damaged − transfers out.
  assert.equal(count('military', 'free'), 30 - 4 - 2);
  assert.equal(count('military', 'topBar'), 30 - 4 - 2);
  assert.equal(count('naval', 'free'), 7);
  assert.equal(groupCount(model, GERMANY, 'military', 'built').damaged, 4);
});

test('a country at an hour sums the chosen groups and knows its unbroken stretch', () => {
  const model = factoryModel(REPLY);
  const at = factoriesAt(model, 'GER', 100, GROUPS, 'free')!;
  assert.equal(at.total, 24 + 7 + 22);
  assert.deepEqual(at.groups.map(group => group.key), GROUPS);
  assert.equal(at.stretch, 100);
  assert.equal(factoriesAt(model, 'GER', 103, GROUPS, 'built')!.stretch, 100);
  // After the missing hour 104, and again after another country takes the tag.
  assert.equal(factoriesAt(model, 'GER', 105, GROUPS, 'built')!.stretch, 105);
  assert.equal(factoriesAt(model, 'GER', 107, GROUPS, 'built')!.stretch, 107);
  assert.equal(factoriesAt(model, 'GER', 104, GROUPS, 'built'), null);
  assert.equal(factoriesAt(model, 'XXX', 100, GROUPS, 'built'), null);
  assert.equal(factoriesAt(model, 'GER', 100, ['civilian'], 'built')!.total, 40);
});

test('Built shows each type total once and keeps its ownership and damage breakdown, without lines that are zero', () => {
  const model = factoryModel(REPLY);
  const group = REPLY.groups.find(group => group.key === 'civilian')!;
  const built = groupRows(model, GERMANY, group, 'built', undefined);
  assert.equal(built[0].value, '40');
  assert.ok(!built.some(row => row.label === 'Total'));
  for (const label of ['Owned', 'From occupation', 'Damaged']) assert.ok(built.some(row => row.label === label));
  assert.ok(!built.some(row => row.label === 'From subjects'));
  for (const mode of ['free', 'topBar'] as const) {
    const rows = groupRows(model, GERMANY, group, mode, undefined);
    assert.equal(rows.find(row => row.label === 'Total')?.value, '40');
  }
});

test('counts over time merge equal neighbours but keep gaps and new lines', () => {
  const model = factoryModel(REPLY);
  assert.deepEqual(countRuns(model, 'GER', ['civilian'], 'built'), [
    { firstHour: 100, lastHour: 101, value: 40, breakBefore: true },
    { firstHour: 102, lastHour: 103, value: 41 },
    { firstHour: 105, lastHour: 106, value: 41, breakBefore: true },
    { firstHour: 107, lastHour: 107, value: 41, breakBefore: true },
  ]);
  assert.deepEqual(countRuns(model, 'ITA', GROUPS, 'built', 'military'), [{ firstHour: 100, lastHour: 107, value: 9, breakBefore: true }]);
});

test('types come from the content: named by their buildings, chosen by group, all when none are chosen', () => {
  const model = factoryModel(REPLY);
  assert.equal(groupName(REPLY.groups[2]), 'Civilian Factory · Building name unavailable');
  assert.equal(groupName(REPLY.groups[1]), 'Factory group name unavailable');
  assert.deepEqual(chosenGroups(model, ['civilian']), ['civilian']);
  assert.deepEqual(chosenGroups(model, []), GROUPS);
  assert.deepEqual(chosenGroups(model, ['not_a_group']), GROUPS);
});

test('a modded content stack: its own factory types appear, removed ones vanish and tags stay opaque', () => {
  // A mod renamed the military factory, added a type of its own, left one unnamed and
  // removed the dockyard.
  const modded = structuredClone(REPLY);
  modded.groups = [
    { key: 'military', flag: 'military_production', buildings: [{ token: 'arms_factory', name: 'Arms Plant' }, { token: 'tank_works', name: 'Tank Works' }] },
    { key: 'naval', flag: 'naval_production', buildings: [] },
    { key: 'civilian', flag: 'general_production', buildings: [{ token: 'industrial_complex', name: 'Civilian Factory' }, { token: 'workshop', name: null }] },
  ];
  assert.deepEqual(typeChoices(modded), [
    { value: 'military', label: 'Arms Plant · Tank Works' },
    { value: 'naval', label: 'Factory group name unavailable' },
    { value: 'civilian', label: 'Civilian Factory · Building name unavailable' },
  ]);
  assert.deepEqual(typeChoices(undefined), []);
  // Dynamic, spaced and odd tags are opaque: asked for and ranked as they are.
  const odd = structuredClone(REPLY);
  odd.series[0].tag = 'MY TAG';
  odd.series[1].tag = 'D01';
  const model = factoryModel(odd);
  assert.deepEqual(factoriesRequest('1', { tags: ['MY TAG', 'D01', 'a"b'] }, 'free', []), { branchId: '1', tags: ['MY TAG', 'D01', 'a"b'], top: null });
  assert.deepEqual(rankFactories(model, ['D01', 'MY TAG'], 100, GROUPS, 'built', 2).map(entry => entry.tag), ['MY TAG', 'D01']);
  assert.equal(mixCountry({ kind: 'tags', tags: ['MY TAG'] }, null, []), 'MY TAG');
});

test('the tooltip gives the game’s breakdown without lines that are zero, then the counts alone', () => {
  const model = factoryModel(REPLY);
  const at = factoriesAt(model, 'GER', 100, ['civilian'], 'free')!;
  const [compact, counts] = [...factoryTooltips(model, 'Germany', at, '1 Jan 1936', 'free', new Map())];
  assert.equal(compact.title, 'Germany');
  assert.equal(compact.subtitle, '1 Jan 1936 · Available');
  const labels = (rows: typeof compact.rows) => rows.map(row => `${row.label} ${row.value}`);
  assert.deepEqual(labels(compact.rows), [
    'Civilian Factory · Building name unavailable 22', 'Total 40', 'Owned 30', 'From occupation +8', 'Trade exports +3', 'Damaged −3',
    'Consumer Goods −5', 'Trade imports −4', 'National Projects −2', 'License imports −1', 'Market Contracts −1', 'Given to other countries −2',
  ]);
  assert.deepEqual(labels(counts.rows), ['Civilian Factory · Building name unavailable 22']);
  assert.equal(formatCount(-1234), '−1,234');
  // A deduction is shown as what it does to the count: a recorded damaged total of −1
  // adds one, so it reads +1, never "−-1".
  const odd = structuredClone(REPLY);
  odd.series[0].runs[0].values = values({ 'civilian:total': 10, 'civilian:damaged': -1 });
  const oddModel = factoryModel(odd);
  const oddAt = factoriesAt(oddModel, 'GER', 100, ['civilian'], 'free')!;
  assert.equal(oddAt.total, 11);
  const [shown] = [...factoryTooltips(oddModel, 'Germany', oddAt, 'day', 'free', new Map())];
  const damaged = shown.rows.find(row => row.label === 'Damaged')!;
  assert.deepEqual([damaged.value, damaged.tone], ['+1', 'positive']);
  assert.deepEqual(compact.rows.find(row => row.label === 'Damaged')!.tone, 'negative');
});

test('top N ranks by the count shown, ties by tag, and leaves out a country not recorded at the hour', () => {
  const model = factoryModel(REPLY);
  const tags = (hour: number, mode: CountMode, count: number) => rankFactories(model, model.tags, hour, GROUPS, mode, count).map(entry => entry.tag);
  // Built: Germany 78 against Italy 21.
  assert.deepEqual(tags(100, 'built', 10), ['GER', 'ITA']);
  assert.deepEqual(tags(100, 'built', 1), ['GER']);
  // Hour 104 is a gap in Germany's record: only Italy is ranked, never Germany at zero.
  assert.deepEqual(tags(104, 'built', 10), ['ITA']);
  // Available: Germany 53 against Italy 21, still first; civilian only, Germany 41 against 12.
  assert.deepEqual(rankFactories(model, model.tags, 100, ['civilian'], 'free', 10).map(entry => [entry.tag, entry.at.total]), [['GER', 22], ['ITA', 12]]);
  const tie = structuredClone(REPLY);
  tie.series[1].tag = 'AAA';
  tie.series[1].runs[0].values = GERMANY;
  assert.deepEqual(rankFactories(factoryModel(tie), ['GER', 'AAA'], 100, GROUPS, 'free', 10).map(entry => entry.tag), ['AAA', 'GER']);
  // Ties break as the product's BTreeMap breaks them, by bytes: AAA before aaa, so
  // the country ranked first is the same whatever N is.
  tie.series[0].tag = 'aaa';
  const first = (count: number) => rankFactories(factoryModel(tie), ['aaa', 'AAA'], 100, GROUPS, 'free', count)[0].tag;
  assert.deepEqual([first(1), first(2)], ['AAA', 'AAA']);
});

test('race entries preserve the count, damage, selected types and unbroken stretch', () => {
  const model = factoryModel(REPLY);
  const look = (mode: CountMode, split = false) => ({ mode, split });
  const entry = (mode: CountMode, split?: boolean) => raceEntries(rankFactories(model, ['GER'], 100, GROUPS, mode, 1), look(mode, split))[0];
  assert.deepEqual(entry('built'), { tag: 'GER', value: 78, label: '78', stretch: 100, hatched: 8 });
  assert.deepEqual(entry('free'), { tag: 'GER', value: 53, label: '53', stretch: 100, hatched: 0 });
  assert.deepEqual(entry('topBar'), { tag: 'GER', value: 66, label: '66', stretch: 100, hatched: 0 });
  assert.deepEqual(entry('built', true).parts, [
    { id: 'military', value: 30, hatched: 4, hatchInside: true },
    { id: 'naval', value: 8, hatched: 1, hatchInside: true },
    { id: 'civilian', value: 40, hatched: 3, hatchInside: true },
  ]);
  const negative = structuredClone(REPLY);
  negative.series[0].runs[0].values = values({ 'military:total': 1, 'military:damaged': 2, 'military:toOthers': 1, 'civilian:total': 10 });
  const at = (groups: FactoryGroupKey[], split: boolean) =>
    raceEntries(rankFactories(factoryModel(negative), ['GER'], 100, groups, 'free', 1), look('free', split))[0];
  assert.deepEqual(at(['military'], false), { tag: 'GER', value: -2, label: '−2', stretch: 100, hatched: 0 });
  const parts = at(['military', 'civilian'], true);
  assert.deepEqual([parts.value, parts.label], [8, '8']);
  assert.deepEqual(parts.parts, [
    { id: 'military', value: -2, hatched: 0, hatchInside: true },
    { id: 'civilian', value: 10, hatched: 0, hatchInside: true },
  ]);
  assert.deepEqual([101, 102, 105, 107].map(hour =>
    raceEntries(rankFactories(model, ['GER'], hour, ['civilian'], 'free', 1), look('free'))[0].stretch), [100, 100, 105, 107]);
});

test('the request names only what is drawn: the top N of the chosen types, or the chosen countries', () => {
  assert.deepEqual(factoriesRequest('1', { tags: [], top: 10 }, 'free', ['civilian', 'not_a_group']),
    { branchId: '1', tags: [], top: { count: 10, mode: 'free', groups: ['civilian'] } });
  assert.deepEqual(factoriesRequest('1', { tags: [], top: 5 }, 'topBar', []), { branchId: '1', tags: [], top: { count: 5, mode: 'topBar', groups: [] } });
  assert.deepEqual(factoriesRequest('1', { tags: ['GER', 'ITA'] }, 'built', ['naval']), { branchId: '1', tags: ['GER', 'ITA'], top: null });
});

test('a top-N reply shows each country only while it was among them', () => {
  const top = structuredClone(REPLY);
  top.series[1].membership = [{ firstHour: 100, lastHour: 102 }];
  top.series[1].runs = [{ ...top.series[1].runs[0], lastHour: 102 }];
  assert.equal(validateFactoriesReply(top), top);
  const model = factoryModel(top);
  assert.deepEqual(model.series.get('ITA')!.membership, [{ firstHour: 100, lastHour: 102 }]);
  assert.deepEqual(rankFactories(model, model.tags, 103, GROUPS, 'built', 10).map(entry => entry.tag), ['GER']);
  const unordered = structuredClone(top);
  unordered.series[1].membership = [{ firstHour: 100, lastHour: 102 }, { firstHour: 101, lastHour: 103 }];
  assert.throws(() => validateFactoriesReply(unordered));
});

test('one country’s mix keeps to the group: the member ranked first then, cut to its member hours, negative counts kept', () => {
  const long = structuredClone(REPLY);
  long.series = [{ tag: 'GER', membership: null, segments: [{ firstHour: 100, lastHour: 200 }], runs: [{ firstHour: 100, lastHour: 200, startsSegment: true,
    values: values({ 'civilian:total': 10, 'military:total': 1, 'military:damaged': 3 }) }] }];
  const model = factoryModel(long);
  // Germany is a member (say, human-controlled) only from 120 to 140.
  const membership = [{ tag: 'GER', firstHour: 120, lastHour: 140 }];
  const resolved = { ...resolveCountries({ kind: 'tags', tags: ['GER'] }, [], undefined, 200, 30), membership: new Map([['GER', membership]]) };
  const group = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
  const rankedAt = (hour: number) => rankFactories(model, rankedCandidates(resolved, hour, model.tags), hour, GROUPS, 'free', 1);
  assert.equal(mixCountry(group, 'ITA', rankedAt(130)), 'GER');
  // At 180 no member was recorded: nobody, never a country outside the group.
  assert.equal(mixCountry(group, 'ITA', rankedAt(180)), null);
  assert.equal(mixCountry({ kind: 'selected' }, 'ITA', []), 'ITA');
  assert.equal(mixCountry({ kind: 'tags', tags: ['FRA', 'GER'] }, 'ITA', []), 'FRA');
  // Its types are cut to its member hours, and a negative count stays negative.
  const whole = { firstHour: 0, lastHour: 1000 };
  const bands = mixBands(model, 'GER', ['military', 'civilian'], 'free', membership, whole);
  assert.deepEqual(bands, [[{ firstHour: 120, lastHour: 140, value: -2 }], [{ firstHour: 120, lastHour: 140, value: 10 }]]);
  assert.deepEqual(mixBands(model, 'GER', ['civilian'], 'free', null, whole), [[{ firstHour: 100, lastHour: 200, value: 10, breakBefore: true }]]);
});

test('lines and mix bands keep to the date range shown, however far an axis is dragged', () => {
  const long = structuredClone(REPLY);
  long.series = [{ tag: 'GER', membership: null, segments: [{ firstHour: 100, lastHour: 200 }], runs: [
    { firstHour: 100, lastHour: 125, startsSegment: true, values: values({ 'civilian:total': 10 }) },
    { firstHour: 126, lastHour: 200, startsSegment: false, values: values({ 'civilian:total': 12, 'naval:total': 3 }) },
  ] }];
  const model = factoryModel(long);
  const range = { firstHour: 120, lastHour: 130 };
  assert.deepEqual(lineRuns(model, 'GER', ['civilian'], 'built', null, range), [
    { firstHour: 120, lastHour: 125, value: 10 }, { firstHour: 126, lastHour: 130, value: 12 },
  ]);
  // Nothing past the range is left to fit the value axis or be hovered.
  const bands = mixBands(model, 'GER', ['naval', 'civilian'], 'built', null, range);
  assert.ok(bands.flat().every(run => run.firstHour >= 120 && run.lastHour <= 130), JSON.stringify(bands));
  // A group's hours and the range both cut.
  assert.deepEqual(lineRuns(model, 'GER', ['civilian'], 'built', [{ firstHour: 128, lastHour: 500 }], range), [{ firstHour: 128, lastHour: 130, value: 12 }]);
});

test('the corner date is a race setting, off by default', () => {
  const field = OPTIONS.find(item => item.key === 'dateOverlay')!;
  assert.deepEqual({ ...field, visibleWhen: DATE_OVERLAY_OPTION.visibleWhen }, DATE_OVERLAY_OPTION);
  assert.equal(resolveOptions(OPTIONS, {}).dateOverlay, false);
  assert.equal(field.visibleWhen?.(resolveOptions(OPTIONS, {})), true);
  assert.equal(field.visibleWhen?.(resolveOptions(OPTIONS, { view: 'lines' })), false);
  assert.equal(field.visibleWhen?.(resolveOptions(OPTIONS, { show: 'breakdown' })), false);
  assert.equal(resolveOptions(OPTIONS, {}).count, 'free');
  assert.equal(resolveOptions(OPTIONS, {}).view, 'race');
});

test('a race defaults to the top 10 of all countries and ranks every country for a top-N group', () => {
  const widget = createWidget('factories', 'main', 'w');
  assert.deepEqual(widget.countries, { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false });
  const resolved = resolveCountries(widget.countries, [], undefined, 200, 30);
  assert.equal(resolved.top, 10);
  assert.equal(resolved.reason, null);
  assert.deepEqual(rankedCandidates(resolved, 100, ['GER', 'ITA']), ['GER', 'ITA']);
  // A group counted at each hour offers only its members at that hour.
  const chosen = resolveCountries({ kind: 'tags', tags: ['GER'] }, [], undefined, 200, 30);
  assert.deepEqual(rankedCandidates(chosen, 100, ['GER', 'ITA']), ['GER']);
  const counted = { ...resolveCountries({ kind: 'tags', tags: ['GER', 'ITA'] }, [], undefined, 200, 30),
    membership: new Map([['GER', [{ tag: 'GER', firstHour: 0, lastHour: 50 }]], ['ITA', [{ tag: 'ITA', firstHour: 0, lastHour: 500 }]]]) };
  assert.deepEqual(rankedCandidates(counted, 100, ['GER', 'ITA']), ['ITA']);
  // Other widgets keep the dashboard's country.
  assert.deepEqual(createWidget('fuel', 'main', 'f').countries, { kind: 'selected' });
});


test('lines outside the top N at branch end still request looks within their unfiltered identities', () => {
  const reply = structuredClone(REPLY);
  reply.series = [{ tag: 'opaque,"tag', membership: [{ firstHour: 150, lastHour: 199 }],
    segments: [{ firstHour: 100, lastHour: 199 }, { firstHour: 200, lastHour: 1000 }],
    runs: [{ firstHour: 150, lastHour: 199, startsSegment: false, values: GERMANY }] }];
  const model = factoryModel(reply);
  // The filtered runs contain nothing at branch end, but its identity remains known.
  assert.equal(factoriesAt(model, 'opaque,"tag', 1000, GROUPS, 'built'), null);
  assert.equal(factorySegmentAt(model, 'opaque,"tag', 1000), 200);
  const lines = [{ tag: 'opaque,"tag', runs: lineRuns(model, 'opaque,"tag', GROUPS, 'built', null, { firstHour: 100, lastHour: 1000 }) }];
  const hours = lineLabelHours(lines, 100, 1000, null);
  assert.equal(hours.get('opaque,"tag'), 199);
  const plan = lookPlan({ firstHour: 100, lastHour: 1000, gaps: [] }, model.tags, 1000,
    (tag, hour) => factorySegmentAt(model, tag, hour), hours);
  // The line's own label precedes the takeover; top membership does not move its sample to 150.
  assert.deepEqual(plan.samples, [{ tag: 'opaque,"tag', hour: 100, stretch: 100 }]);
});
