import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { WidgetStateProps } from '../../components/WidgetState';
import type { TimelineLane } from '../../charts/timeline/Timeline';
import type { AutonomyReply, AutonomyRun, CountryGroupReply } from '../../generated/dashboard';
import { laneStops } from '../../charts/timeline/layout.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { absentAt, autonomyViews, chosenViews, clipSubject, formatProgress, formatScore, laneLabels, levelName, membershipUnknownFor, noLanesMessage, overlordIdentityUnknownFor, partlyIdentified, progressOf, runOf,
  subjectLane, validateAutonomyReply } from './autonomyData.ts';

const run = (firstHour: number, lastHour: number, level: number, scoreRaw: string, lowerRaw: string, upperRaw: string, extra: Partial<AutonomyRun> = {}): AutonomyRun => ({
  firstHour, lastHour, startsSegment: false, overlord: 'GER', level, scoreRaw, lowerRaw, upperRaw, lowest: false, highest: false, ratioRaw: '20000', ...extra,
});
const fixedIdentities = countryIdentityIndex(['D01 mod', 'GER', 'ENG', 'ITA', 'AAA', 'BBB', 'A', 'B'].map((tag, slot) => ({ tag,
  identities: [{ firstHour: 0, lastHour: 100_000, slot, countryId: slot + 10 }] })));
const memberRuns = (entries: [string, { firstHour: number; lastHour: number }[]][]) => new Map(entries.map(([tag, spans]) => [fixedIdentities.keyAt(tag, 0)!, spans]));

test('a release at hour 150 keeps the historical lane and names recorded absence in its pill and description', () => {
  const subject = { tag: 'AAA', runs: [run(100, 149, 0, '15000000', '10000000', '25000000', { startsSegment: true })] };
  const reply: AutonomyReply = { ...REPLY, subjects: [subject], recorded: [{ firstHour: 100, lastHour: 300 }], unidentified: [] };
  const lane = subjectLane(subject, reply.levels);
  assert.equal(lane.spans.length, 1);
  assert.equal(laneLabels(reply, lane, 149, []).pill, 'Serf State');
  for (const hour of [150, 300]) {
    assert.deepEqual(laneLabels(reply, lane, hour, []), { label: 'Not a subject shown here', pill: 'Not a subject here', description: 'Not a subject shown here' });
  }
  const missing = { ...reply, recorded: [{ firstHour: 100, lastHour: 149 }] };
  assert.deepEqual(laneLabels(missing, lane, 300, []), { label: 'Not recorded', pill: 'Not recorded', description: 'Not recorded' });
  const unidentified = { ...reply, unidentified: [{ firstHour: 300, lastHour: 300 }] };
  assert.equal(laneLabels(unidentified, lane, 300, []).pill, 'Not known');
  assert.equal(laneLabels(unidentified, lane, 300, []).description, absentAt(unidentified, 300, []).text);
  const unobserved = [{ firstHour: 300, lastHour: 300 }];
  assert.equal(laneLabels(reply, lane, 300, unobserved).pill, 'Membership not recorded');
  assert.equal(laneLabels(reply, lane, 300, unobserved).description, absentAt(reply, 300, unobserved).text);
});

// A mod's own levels, in its own order and number: one the content names, one
// only recorded (named by its id), one the game left unlocalised.
const REPLY: AutonomyReply & { coverage: { firstHour: number; lastHour: number; complete: boolean } } = {
  branchId: '1', scoreScale: 100_000,
  coverage: { firstHour: 100, lastHour: 160, complete: true },
  levels: [
    { id: 'mod_serf', name: 'Serf State', defined: true },
    { id: 'mod_vassal', name: null, defined: true },
    { id: 'mod_ally', name: 'Ally', defined: false },
  ],
  subjects: [
    { tag: 'D01 mod', runs: [
      run(100, 110, 0, '15000000', '10000000', '25000000', { startsSegment: true, lowest: true }),
      run(111, 120, 0, '20000000', '10000000', '25000000', { lowest: true }),
      // Moved up a level at 121.
      run(121, 130, 1, '25000000', '25000000', '50000000'),
      // A gap, then back at the lowest level: no change is marked across a gap.
      run(140, 150, 0, '12000000', '10000000', '25000000', { startsSegment: true, lowest: true }),
    ] },
    { tag: 'ITA', runs: [
      run(100, 150, 2, '51250000', '40000000', '50000000', { startsSegment: true, highest: true, overlord: 'ENG' }),
      run(151, 160, 2, '51250000', '40000000', '40000000', { highest: true, overlord: null, ratioRaw: null }),
    ] },
  ],
  recorded: [{ firstHour: 100, lastHour: 130 }, { firstHour: 140, lastHour: 160 }],
  unidentified: [],
  warnings: [],
};

test('renamed subjects and overlords keep one country history, while reused tags stay separate', () => {
  const identities = countryIdentityIndex([
    { tag: 'AAA', identities: [{ firstHour: 100, lastHour: 119, slot: 1, countryId: 10 }, { firstHour: 140, lastHour: 150, slot: 2, countryId: 20 }] },
    { tag: 'BBB', identities: [{ firstHour: 120, lastHour: 150, slot: 1, countryId: 10 }] },
    { tag: 'GER', identities: [{ firstHour: 100, lastHour: 129, slot: 3, countryId: 30 }, { firstHour: 130, lastHour: 150, slot: 4, countryId: 40 }] },
    { tag: 'NEW OWNER', identities: [{ firstHour: 130, lastHour: 150, slot: 3, countryId: 30 }] },
  ]);
  const reply = { ...REPLY, subjects: [
    { tag: 'AAA', runs: [run(100, 119, 0, '1', '0', '10', { startsSegment: true }), run(140, 150, 0, '3', '0', '10', { startsSegment: true })] },
    { tag: 'BBB', runs: [run(120, 129, 0, '2', '0', '10', { startsSegment: true }), run(130, 150, 0, '2', '0', '10', { overlord: 'NEW OWNER' })] },
  ] };
  const views = autonomyViews(reply, identities);
  assert.equal(views.length, 2);
  const original = identities.keyAt('AAA', 100)!;
  const selected = chosenViews(views, new Set([original]), true, identities);
  assert.equal(selected.length, 1);
  const lane = subjectLane(selected[0], reply.levels);
  assert.equal(lane.id, original);
  assert.deepEqual(lane.stretches, [{ firstHour: 100, lastHour: 150 }]);
  assert.deepEqual(chosenViews(views, new Set([identities.keyAt('NEW OWNER', 150)!]), false, identities).map(view => view.countryId), [original]);
  assert.deepEqual(chosenViews(views, new Set([identities.keyAt('GER', 150)!]), false, identities).map(view => view.countryId), [identities.keyAt('AAA', 140)!]);
  const membership = new Map([[identities.keyAt('GER', 100)!, [{ firstHour: 100, lastHour: 145 }]]]);
  assert.equal(clipSubject(selected[0], false, membership, [], identities).runs.at(-1)?.lastHour, 145);
});

test('native manual holders clip reused tags and gaps without taking unrelated aliases', () => {
  const identities = countryIdentityIndex([
    { tag: 'X', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 30, slot: 1, countryId: 20 }] },
    { tag: 'Y', identities: [{ firstHour: 10, lastHour: 19, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 30, slot: 2, countryId: 30 }] },
    { tag: 'GER', identities: [{ firstHour: 0, lastHour: 30, slot: 3, countryId: 40 }] },
  ]);
  const reply = { ...REPLY, subjects: ['X', 'Y'].map(tag => ({ tag, runs: [run(0, 30, 0, '1', '0', '10', { startsSegment: true })] })) };
  const views = autonomyViews(reply, identities);
  const chosen = chosenViews(views, identities.selected(['X']), true, identities);
  assert.equal(chosen.length, 2);
  assert.deepEqual(chosen.map(subject => subject.runs.map(run => [run.firstHour, run.lastHour])), [[[0, 9], [10, 19]], [[20, 30]]]);
  assert.deepEqual(subjectLane(chosen[0], reply.levels).stretches, [{ firstHour: 0, lastHour: 19 }]);
  const membership = new Map([[identities.keyAt('Y', 10)!, [{ firstHour: 12, lastHour: 15 }]]]);
  const clipped = clipSubject(chosen[0], true, membership, [], identities);
  assert.deepEqual(clipped.runs.map(run => [run.firstHour, run.lastHour]), [[12, 15]]);
  assert.deepEqual(laneLabels(reply, subjectLane(chosen[0], reply.levels), 20, [], false),
    { label: 'Country unavailable at this time.', pill: 'Not recorded', description: 'Country unavailable at this time.' });
});

test('native overlord replacement splits selection even when the subject and tag stay the same', () => {
  const identities = countryIdentityIndex([
    { tag: 'X', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }] },
    { tag: 'GER', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 30, slot: 1, countryId: 20 }] },
    { tag: 'SUB', identities: [{ firstHour: 0, lastHour: 30, slot: 2, countryId: 30 }] },
  ]);
  const views = autonomyViews({ ...REPLY, subjects: [{ tag: 'SUB', runs: [run(0, 30, 0, '1', '0', '10')] }] }, identities);
  // Subject values remain recorded through unknown overlord spans.
  assert.deepEqual(views[0].runs.map(run => [run.firstHour, run.lastHour]), [[0, 9], [10, 19], [20, 30]]);
  assert.equal(chosenViews(views, identities.selected(['X']), false, identities).length, 1);
  const membership = new Map([[identities.keyAt('GER', 0)!, [{ firstHour: 0, lastHour: 30 }]]]);
  assert.deepEqual(clipSubject(views[0], false, membership, [], identities).runs.map(run => [run.firstHour, run.lastHour]), [[0, 9]]);
});

test('known subjects retain their values when an unresolved overlord prevents proving selected absence', () => {
  const identities = countryIdentityIndex([
    { tag: 'X', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 30, slot: 1, countryId: 10 }] },
    { tag: 'SUB', identities: [{ firstHour: 0, lastHour: 30, slot: 2, countryId: 20 }] },
    { tag: 'Other', identities: [{ firstHour: 0, lastHour: 30, slot: 3, countryId: 30 }] },
  ]);
  const reply = { ...REPLY, subjects: [{ tag: 'SUB', runs: [run(10, 19, 0, '123', '0', '1000', { overlord: 'X' })] }] };
  const views = autonomyViews(reply, identities);
  assert.equal(views.length, 1);
  assert.equal(views[0].runs[0].scoreRaw, '123');
  assert.deepEqual(chosenViews(views, identities.selected(['X']), false, identities), []);
  assert.equal(overlordIdentityUnknownFor(views[0], identities), true);
  assert.equal(chosenViews(views, identities.selected(['SUB']), true, identities).length, 1);
  for (const overlord of ['Other', null]) {
    const known = { ...views[0], runs: views[0].runs.map(run => ({ ...run, overlord })) };
    assert.equal(overlordIdentityUnknownFor(known, identities), false);
    assert.deepEqual(chosenViews([known], identities.selected(['X']), false, identities), []);
  }
});

test('actual Autonomy refuses false overlord absence while keeping known subject values and proved empty selections', async () => {
  const data = { ...REPLY, coverage: { firstHour: 0, lastHour: 30, complete: true }, recorded: [{ firstHour: 0, lastHour: 30 }],
    subjects: [{ tag: 'SUB', runs: [run(10, 19, 0, '123', '0', '1000', { overlord: 'X' })] }] };
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [
      { tag: 'X', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }, { firstHour: 20, lastHour: 30, slot: 1, countryId: 10 }] },
      { tag: 'SUB', identities: [{ firstHour: 0, lastHour: 30, slot: 2, countryId: 20 }] },
      { tag: 'Other', identities: [{ firstHour: 0, lastHour: 30, slot: 3, countryId: 30 }] },
    ], unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
  const fixture: { data: AutonomyReply; groups: CountryGroupReply; state?: WidgetStateProps; lanes?: TimelineLane[] } = { data, groups };
  const host = globalThis as typeof globalThis & { __autonomyR2?: typeof fixture };
  host.__autonomyR2 = fixture;
  const replacements: Record<string, string> = {
    '/dashboard/preparedSeries.ts': `export const usePreparedWindow = (_client, _source, product, params) => {
      globalThis.__autonomyR2.request = { product, params };
      const data = globalThis.__autonomyR2.data;
      return { data: { ...data, metadata: data, exactHours: [30], summaries: {} }, geometryReady: true, onSummaryView() {}, navigate: async () => null };
    };`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__autonomyR2.groups });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 25;',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ surface: '#111111', text: '#ffffff', accent: '#aaaaaa' });",
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag: () => undefined, revision: 0 });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null });',
    '/components/WidgetState.tsx': 'export default props => { globalThis.__autonomyR2.state = props; return props.message ?? props.label; };',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__autonomyR2.lanes = props.lanes; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'autonomy-native-unknown', enforce: 'pre', transform(_code, id) {
      const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/autonomy/AutonomyWidget.tsx');
    const props = { widget: { id: 'autonomy', kind: 'autonomy', trackIds: ['main'], countries: { kind: 'tags', tags: ['X'] }, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 0, lastHour: 30 }, axisOffset: 0 }],
        clock: { getHour: () => 25, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const currentState = () => fixture.state;
    const render = () => { fixture.state = undefined; fixture.lanes = undefined; return renderToStaticMarkup(createElement(Widget, props)); };
    const unresolved = render();
    assert.equal(fixture.state?.kind, 'unavailable');
    assert.match(unresolved, /Subject data is unavailable for these countries/);
    assert.doesNotMatch(unresolved, /had no subjects/);
    props.widget.countries = { kind: 'tags', tags: ['SUB'] }; props.widget.options = { countriesAre: 'subjects' };
    render();
    assert.equal(fixture.state, undefined);
    assert.equal(fixture.lanes?.length, 1);
    assert.equal(fixture.lanes?.[0].spans[0].firstHour, 10);
    assert.equal(fixture.lanes?.[0].spans[0].lastHour, 19);
    props.widget.countries = { kind: 'tags', tags: ['X'] }; props.widget.options = {};
    fixture.data = { ...data, subjects: [{ tag: 'SUB', runs: [run(10, 19, 0, '123', '0', '1000', { overlord: 'Other' })] }] };
    render();
    assert.equal(currentState()?.kind, 'empty', 'a known unrelated overlord proves the selected overlord has no matching subjects');
    fixture.data = { ...data, subjects: [{ tag: 'SUB', runs: [run(0, 9, 0, '100', '0', '1000', { overlord: 'X' }), ...data.subjects[0].runs] }] };
    assert.doesNotMatch(render(), /matching subjects is incomplete in those hours/);
    assert.equal(fixture.lanes?.length, 1, 'a proved matching span preserves the subject history');
    assert.equal(fixture.lanes?.[0].spans.at(-1)?.lastHour, 19, 'known autonomy values survive an unknown overlord');
  } finally { delete host.__autonomyR2; await vite.close(); }
});

test('5,000 native subject holders remain selectable without a country cap', () => {
  const countries = Array.from({ length: 5000 }, (_, slot) => ({ tag: `mod ${slot}`, identities: [{ firstHour: 0, lastHour: 1, slot, countryId: slot + 10 }] }));
  const identities = countryIdentityIndex(countries);
  const views = autonomyViews({ ...REPLY, subjects: countries.map(country => ({ tag: country.tag, runs: [run(0, 1, 0, '1', '0', '10', { overlord: null })] })) }, identities);
  assert.equal(chosenViews(views, identities.selected(countries.map(country => country.tag)), true, identities).length, 5000);
});

test('the reply is checked, and runs naming no level are refused', () => {
  assert.equal(validateAutonomyReply(REPLY), REPLY);
  assert.throws(() => validateAutonomyReply({ ...REPLY, subjects: [{ tag: 'X', runs: [run(1, 2, 3, '1', '0', '2')] }] }));
  assert.throws(() => validateAutonomyReply({ ...REPLY, levels: [...REPLY.levels, REPLY.levels[0]] }));
  assert.throws(() => validateAutonomyReply({ ...REPLY, scoreScale: 3 }));
});

test('splitting at an overlord tag change keeps a gap break only on the original first piece', () => {
  const identities = countryIdentityIndex([
    { tag: 'AAA', identities: [{ firstHour: 100, lastHour: 150, slot: 1, countryId: 10 }] },
    { tag: 'GER', identities: [{ firstHour: 100, lastHour: 119, slot: 2, countryId: 20 }] },
    { tag: 'NEW OWNER', identities: [{ firstHour: 120, lastHour: 150, slot: 2, countryId: 20 }] },
  ]);
  const reply = { ...REPLY, subjects: [{ tag: 'AAA', runs: [run(100, 150, 0, '1', '0', '10', { startsSegment: true })] }] };
  const [view] = autonomyViews(reply, identities);
  assert.deepEqual(view.runs.map(run => [run.firstHour, run.lastHour, run.startsSegment]), [[100, 119, true], [120, 150, false]]);
  const lane = subjectLane(view, reply.levels);
  assert.deepEqual(lane.stretches, [{ firstHour: 100, lastHour: 150 }]);
  assert.deepEqual(lane.progress.map(run => run.breakBefore), [true, false]);
});

test('levels show localized titles and keep missing, empty and key-equal names distinct', () => {
  assert.deepEqual(REPLY.levels.map(levelName), ['Serf State', 'Level name unavailable', 'Ally']);
  assert.equal(levelName(undefined), 'Unknown level');
  assert.equal(levelName({ id: 'synthetic', name: '', defined: true }), '');
  assert.equal(levelName({ id: 'synthetic', name: 'synthetic', defined: true }), 'synthetic');
});

test('a lane bands each level for the whole span it applied, marks each change and never joins across a gap', () => {
  const lane = subjectLane(REPLY.subjects[0], REPLY.levels);
  assert.deepEqual(lane.spans.map(span => [span.firstHour, span.lastHour, span.label]), [[100, 120, 'Serf State'], [121, 130, 'Level name unavailable'], [140, 150, 'Serf State']]);
  assert.deepEqual([...lane.spanLevels.values()], [0, 1, 0]);
  assert.deepEqual(lane.changes, [121]);
  assert.deepEqual(lane.stretches, [{ firstHour: 100, lastHour: 130 }, { firstHour: 140, lastHour: 150 }]);
  assert.deepEqual(lane.progress.map(item => [item.firstHour, item.value, item.breakBefore]),
    // Exact to a billionth, rounded toward zero.
    [[100, 1 / 3, true], [111, 2 / 3, false], [121, 0, false], [140, 2 / 15, true]].map(([hour, value, fresh]) => [hour, Math.floor(Number(value) * 1e9) / 1e9, fresh]));
  assert.equal(runOf(lane, 135), null);
  assert.equal(runOf(lane, 125)?.level, 1);
});

test('progress sits between the thresholds, may pass the top one at the top level, and is undefined without a span', () => {
  const [top, flat] = REPLY.subjects[1].runs;
  assert.equal(progressOf(top), 1.125);
  assert.equal(formatProgress(top), '112.5%');
  assert.equal(progressOf(flat), null);
  assert.equal(formatProgress(flat), 'Progress unavailable');
  assert.equal(formatScore(REPLY, top.scoreRaw), '512.5');
  // A lane without defined progress there draws no line (a gap), but keeps its band.
  const lane = subjectLane(REPLY.subjects[1], REPLY.levels);
  assert.deepEqual(lane.progress.map(item => item.lastHour), [150]);
  assert.deepEqual(lane.spans.map(span => [span.firstHour, span.lastHour]), [[100, 160]]);
});

test('the keyboard stops at every change inside a level, not only where the level starts', () => {
  // The level holds 100–120, but the score changes at 111.
  const lane = subjectLane(REPLY.subjects[0], REPLY.levels);
  assert.deepEqual(lane.spans[0].stops, [111]);
  assert.deepEqual(laneStops(lane.spans, 100, 150, null).map(stop => stop.hour), [100, 111, 121, 140]);
});

test('a group counted at each hour shows a subject only while it, or its overlord, belongs; unknown membership is cut out', () => {
  const subject = { ...REPLY.subjects[0], countryId: fixedIdentities.keyAt(REPLY.subjects[0].tag, 100)! };
  // As a chosen subject: a member from 110 (as the human players group can become), so nothing shows before.
  const member = memberRuns([['D01 mod', [{ firstHour: 110, lastHour: 150 }]]]);
  const own = clipSubject(subject, true, member, []);
  assert.deepEqual(own.runs.map(item => [item.firstHour, item.lastHour]), [[110, 110], [111, 120], [121, 130], [140, 150]]);
  // As a subject of chosen overlords: only while its overlord (GER) belongs.
  const overlords = memberRuns([['GER', [{ firstHour: 100, lastHour: 115 }]]]);
  assert.deepEqual(clipSubject(subject, false, overlords, [], fixedIdentities).runs.map(item => [item.firstHour, item.lastHour]), [[100, 110], [111, 115]]);
  // Hours whose membership was not recorded are cut out, and break the line.
  const always = memberRuns([['D01 mod', [{ firstHour: 0, lastHour: 1000 }]]]);
  const cut = clipSubject(subject, true, always, [{ firstHour: 105, lastHour: 112 }]);
  assert.deepEqual(cut.runs.map(item => [item.firstHour, item.lastHour]), [[100, 104], [113, 120], [121, 130], [140, 150]]);
  assert.deepEqual(subjectLane(cut, REPLY.levels).stretches, [{ firstHour: 100, lastHour: 104 }, { firstHour: 113, lastHour: 130 }, { firstHour: 140, lastHour: 150 }]);
  // Without a group nothing changes.
  assert.equal(clipSubject(subject, true, null, []), subject);
});

test('a group counted at one moment keeps its countries’ whole history, however its membership went unrecorded elsewhere', () => {
  // AAA is a member at the branch end (110); its subject BBB was recorded at 102–104, when membership was not.
  const bbb = { tag: 'BBB', runs: [run(102, 104, 0, '15000000', '10000000', '25000000', { startsSegment: true, overlord: 'AAA' })] };
  const unobserved = [{ firstHour: 100, lastHour: 105 }];
  assert.equal(clipSubject(bbb, false, null, unobserved), bbb);
  assert.equal(clipSubject(bbb, true, null, unobserved), bbb);
  // Counted at each hour, the same hours are unknown and cut out.
  assert.deepEqual(clipSubject(bbb, false, memberRuns([['AAA', [{ firstHour: 100, lastHour: 110 }]]]), unobserved, fixedIdentities).runs, []);
});

test('autonomy recording spans cannot end before they start', () => {
  assert.throws(() => validateAutonomyReply({ ...REPLY, recorded: [{ firstHour: 5, lastHour: 1 }] }));
});

test('a subject whose overlord changes is cut by each overlord’s own membership, in hour order', () => {
  const subject = { tag: 'D01 mod', runs: [
    run(100, 109, 0, '15000000', '10000000', '25000000', { startsSegment: true }),
    run(110, 119, 0, '15000000', '10000000', '25000000', { overlord: 'ENG' }),
    run(120, 129, 0, '15000000', '10000000', '25000000'),
    run(130, 139, 0, '15000000', '10000000', '25000000', { overlord: null, ratioRaw: null }),
  ] };
  const membership = memberRuns([['GER', [{ firstHour: 105, lastHour: 125 }]], ['ENG', [{ firstHour: 100, lastHour: 200 }]]]);
  assert.deepEqual(clipSubject(subject, false, membership, [], fixedIdentities).runs.map(item => [item.firstHour, item.lastHour, item.overlord, item.startsSegment]),
    [[105, 109, 'GER', false], [110, 119, 'ENG', false], [120, 125, 'GER', false]]);
});

test('cutting a subject by its overlords’ membership counted at each hour keeps their member turns', () => {
  // Two overlords taking turns every other hour, each a member every other turn: as
  // many membership spans as runs, the shape that took seconds when cut run by run.
  const history = (count: number) => {
    const runs = Array.from({ length: count }, (_, index) => run(index * 2, index * 2 + 1, 0, '15000000', '10000000', '25000000',
      { overlord: index % 2 ? 'ENG' : 'GER' }));
    const spans = (parity: number) => Array.from({ length: count / 2 }, (_, index) => index * 4 + parity * 2)
      .filter((_, index) => index % 2 === 0).map(hour => ({ firstHour: hour, lastHour: hour + 1 }));
    return { subject: { tag: 'D01 mod', runs }, membership: memberRuns([['GER', spans(0)], ['ENG', spans(1)]]) };
  };
  const { subject, membership } = history(32_000);
  assert.equal(clipSubject(subject, false, membership, [], fixedIdentities).runs.length, 16_000);
});

test('an hour with a subject or overlord the recording does not name is never called one without subjects', () => {
  const branch = { firstHour: 100, lastHour: 160 };
  const partial: AutonomyReply = { ...REPLY, recorded: [{ firstHour: 100, lastHour: 160 }], unidentified: [{ firstHour: 135, lastHour: 136 }] };
  assert.equal(validateAutonomyReply(partial), partial);
  assert.throws(() => validateAutonomyReply({ ...REPLY, unidentified: [{ firstHour: 5, lastHour: 1 }] }));
  assert.throws(() => validateAutonomyReply({ ...REPLY, unidentified: undefined }));
  assert.equal(partlyIdentified(partial, branch), true);
  assert.equal(partlyIdentified(partial, { firstHour: 100, lastHour: 134 }), false);
  // A lane without a run at such an hour is not known, where elsewhere it is not a subject.
  assert.equal(absentAt(partial, 135, []).text, 'Subject status unknown');
  assert.equal(absentAt(partial, 137, []).text, 'Not a subject shown here');
  assert.equal(absentAt(REPLY, 135, []).text, 'Not recorded');
  assert.equal(absentAt(partial, 135, [{ firstHour: 130, lastHour: 140 }]).text, 'Group membership not recorded');
  // The empty state never claims absence then, by overlord or by subject.
  assert.equal(noLanesMessage(partial, branch), 'Subject data is incomplete for these countries.');
  // Fully identified selections can establish that no subjects were found.
  assert.equal(noLanesMessage({ ...partial, unidentified: [] }, branch), 'No subjects found for these countries.');
  assert.equal(noLanesMessage(REPLY, branch), 'No subjects found for these countries.');
});

test('sampled and pending empty censuses do not claim no subjects in the requested span', () => {
  const within = { firstHour: 100, lastHour: 160 };
  for (const field of ['approximate', 'pending'] as const) {
    const reply = { ...REPLY, subjects: [], [field]: [{ firstHour: 135, lastHour: 136 }] };
    assert.equal(noLanesMessage(reply, within), 'Subject data is still being prepared.');
    assert.equal(noLanesMessage(reply, { firstHour: 100, lastHour: 134 }), 'No subjects found for these countries.');
  }
});

test('a lane cut away where the group’s membership was unknown is said, never called "no subjects"', () => {
  // A belongs to the group 100–105 and its membership is unknown 106–110, when B is its subject.
  const b = { tag: 'B', runs: [run(106, 110, 0, '15000000', '10000000', '25000000', { startsSegment: true, overlord: 'A' })] };
  const reply: AutonomyReply = { ...REPLY, subjects: [b], recorded: [{ firstHour: 100, lastHour: 160 }] };
  const membership = memberRuns([['A', [{ firstHour: 100, lastHour: 105 }]]]);
  const unobserved = [{ firstHour: 106, lastHour: 110 }];
  assert.deepEqual(clipSubject(b, false, membership, unobserved, fixedIdentities).runs, []);
  assert.equal(membershipUnknownFor([b], false, unobserved), true);
  const branch = { firstHour: 100, lastHour: 160 };
  const message = noLanesMessage(reply, branch, membershipUnknownFor([b], false, unobserved));
  assert.equal(message, 'Subject data is incomplete for these countries.');
  assert.doesNotMatch(message, /had no subjects/);
  assert.equal(noLanesMessage(reply, branch, true), 'Subject data is incomplete for these countries.');
  // Membership unknown only where no lane had a run, or only for a run without an overlord, cuts nothing that could show.
  assert.equal(membershipUnknownFor([b], false, [{ firstHour: 111, lastHour: 120 }]), false);
  const released = { tag: 'B', runs: [run(106, 110, 0, '1', '0', '2', { overlord: null })] };
  assert.equal(membershipUnknownFor([released], false, unobserved), false);
  assert.equal(membershipUnknownFor([released], true, unobserved), true);
  assert.equal(noLanesMessage(reply, branch, false), 'No subjects found for these countries.');
});

test('subjects are those of the chosen overlords at any hour, or the chosen subjects, in the reply’s order', () => {
  const views = autonomyViews(REPLY, fixedIdentities);
  const ids = (...tags: string[]) => fixedIdentities.selected(tags);
  assert.deepEqual(chosenViews(views, ids('GER'), false, fixedIdentities).map(subject => subject.tag), ['D01 mod']);
  assert.deepEqual(chosenViews(views, ids('ENG'), false, fixedIdentities).map(subject => subject.tag), ['ITA']);
  assert.deepEqual(chosenViews(views, ids('ITA', 'D01 mod'), true, fixedIdentities).map(subject => subject.tag), ['D01 mod', 'ITA']);
  assert.deepEqual(chosenViews(views, ids('FRA'), false, fixedIdentities), []);
});
