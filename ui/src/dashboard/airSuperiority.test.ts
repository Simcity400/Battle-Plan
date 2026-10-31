import { airChoiceKey, airChoiceToken, heldAirReply, timelineRequestLast } from './airOptions.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { airCountryTags, airScore, airSegmentAt, airSideName, airSideTag, airPercent, airPeriods, airState, airSummaryTooltip, airTooltip, airTooltipTags, cellView, lastRecordedAt, momentReadAt, momentReadHours, momentViews, momentWindows, orderedRegions, sideOrder, timelineViews, validateAirReply } from './airSuperiority.ts';
import type { AirChoices, AirLooks, AirMoment, AirTimeline } from './airSuperiority.ts';
import type { DashboardClient } from './client';
import type { AirSide } from '../generated/dashboard';
import { tooltipText } from '../charts/tooltip.ts';
import { settingsSections } from './settingsSections.ts';
import { definition as regionDefinition } from '../widgets/air-superiority-region/definition.ts';
import { definition as timelineDefinition } from '../widgets/air-superiority-timeline/definition.ts';
import { contentKey, resolveOptions, setToken } from './options.ts';

const span = (firstHour: number, lastHour = firstHour) => ({ firstHour, lastHour });
const common = { branchId: 'mod branch', scale: 100000, dayHours: 24, badBelow: 40000, goodAbove: 60000, warnings: [], segments: {}, anchorReads: {} };
const sides: AirSide[] = [
  { key: 'f:10', faction: '10', colourFaction: '10', tag: null, name: 'Mod tenth faction', leader: 'mod country ! with spaces', coBelligerents: 3 },
  { key: 'f:9', faction: '9', colourFaction: '9', tag: null, name: 'Mod ninth faction', leader: 'Other', coBelligerents: 0 },
  { key: 'w:third', faction: null, colourFaction: null, tag: null, name: 'A coalition', leader: 'Coalition leader', coBelligerents: 0 },
];
const looks: AirLooks = { ...common, name: side => side.name, color: side => side.faction === '10' ? '#336699' : '#993366',
  viewpoint: { name: 'Mod country', tag: 'mod country ! with spaces', color: '#2277cc' }, enemy: '#cc3322' };
function timeline(): AirTimeline {
  return { ...common, view: 'timeline', available: true, firstHour: 0, lastHour: 73, days: [23, null, null, 73], sides,
    excluded: [], theatreReads: [], rows: [{ region: 7000, name: 'A mod region', cells: [[1, { scores: [[0, '4500000'], [1, '3500000'], [2, '2000000']] }], [2, null], [1, ['2', '3']]] }] };
}
function moment(): AirMoment {
  return { ...common, view: 'moment', available: true, firstHour: 0, lastHour: 167, sides, fallback: [],
    recorded: [span(0, 23), span(72, 167)], complete: [span(0, 23), span(72, 167)], wingsKnown: [span(0, 23), span(72, 167)], excluded: [], theatreReads: [],
    regions: [
      { id: 7000, name: 'Mod contested region', active: [span(0, 23), span(72, 167)], runs: [[0, 11, { scores: [[0, '45'], [1, '35'], [2, '20']] }], [12, 23, { scores: [[0, '20'], [1, '80']] }], [72, 167, { scores: [[0, '20'], [1, '80']] }]], wings: [[0, 23, 3], [72, 167, 3]] },
      { id: 8888, name: 'Mod wing region', active: [span(0, 23)], runs: [], wings: [[0, 23, 2]] },
    ] };
}
function choices(): AirChoices {
  return { ...common, view: 'choices', sides, countries: ['mod country ! with spaces'], theatres: [{ id: 'mod theatre with spaces', name: 'Mod theatre' }],
    windows: [span(0, 4999), span(5000, 5299), span(5300, 9999)],
    regions: [{ id: 7000, name: 'Mod contested region' }, { id: 8888, name: 'Mod wing region' }, { id: 9900, name: 'Mod unused region' }] };
}

test('air cards share filters, current-country default and versioned explicit choices', () => {
  for (const definition of [regionDefinition, timelineDefinition]) {
    assert.ok(settingsSections(definition, {}).includes('campaign'));
    assert.equal(definition.countries?.max, 1);
    for (const key of ['sides', 'regions', 'theatres']) assert.equal(definition.options.find(field => field.key === key)?.kind, 'content-set');
    assert.ok(definition.options.filter(field => field.kind === 'content-set').every(field => !('max' in field)));
    const viewpoint = definition.options.find(field => field.key === 'viewpoint');
    assert.match(viewpoint?.kind === 'content-choice' ? viewpoint.automatic : '', /current country/);
  }
  assert.equal(resolveOptions(regionDefinition.options, {}).viewpoint, '');
  assert.equal(contentKey(setToken('country:mod country ! with spaces')), 'country:mod country ! with spaces');
});

test('timeline projects only selected rows, expands RLE once and shades missing calendar days', () => {
  const reply = validateAirReply(timeline()) as AirTimeline;
  const projected = timelineViews(reply, looks);
  assert.equal(projected.rows.length, 1);
  assert.deepEqual(projected.rows[0].cells[0].parts.map(part => part.id), ['f:9', 'f:10', 'w:third']);
  assert.equal(projected.rows[0].cells[1], projected.rows[0].cells[2]);
  assert.deepEqual(airPeriods(reply), [
    { firstHour: 0, lastHour: 23, readHour: 23 }, { firstHour: 24, lastHour: 47, readHour: null },
    { firstHour: 48, lastHour: 71, readHour: null }, { firstHour: 72, lastHour: 73, readHour: 73 },
  ]);
});

test('moment lookup keeps equal values separated by a gap and changes only at a read boundary', () => {
  const reply = validateAirReply(moment()) as AirMoment;
  const reads = momentReadHours(reply);
  assert.deepEqual(reads, [0, 12, 24, 72]);
  assert.equal(momentReadAt(reads, 11), 0);
  assert.equal(momentReadAt(reads, 12), 12);
  assert.equal(momentReadAt(reads, 71), 24);
  const before = momentViews(reply, 23, looks)[0].view;
  const after = momentViews(reply, 72, looks)[0].view;
  assert.deepEqual(before.parts.map(part => part.score), after.parts.map(part => part.score));
  assert.equal(momentViews(reply, 24, looks)[0].view.state, 'gap');
  assert.equal(after.hour, 72);
  const windows = choices().windows;
  assert.deepEqual(momentWindows(windows, 3749), [0]);
  assert.deepEqual(momentWindows(windows, 3750), [0, 1]);
  assert.deepEqual(momentWindows(windows, 5000), [1]);
  assert.deepEqual(momentWindows(windows, 5225), [1, 2]);
  assert.deepEqual(momentWindows(windows, 9999), [2]);
  assert.deepEqual(momentWindows([], 126), []);
});

test('unchanged scores still resolve a new side name and supplying leader', () => {
  const reply = moment();
  reply.sides = [{ ...sides[0], reads: [
    [-5, 'Earlier name', 'Earlier leader', null, 3, [], '10'],
    [7, 'Renamed faction', 'New leader', null, 3, [], '10'],
    [180, 'Outside this window', 'Later leader', null, 3, [], '10'],
  ] }, ...sides.slice(1)];
  const reads = momentReadHours(reply);
  assert.ok(reads.includes(7));
  assert.ok(!reads.includes(-5));
  assert.ok(!reads.includes(180));
  const at = momentReadAt(reads, 8)!;
  const part = momentViews(reply, at, looks)[0].view.parts.find(part => part.id === 'f:10')!;
  assert.equal(part.label, 'Renamed faction');
  assert.equal(part.tag, 'New leader');
  assert.equal(part.score, 45n);
});

test('equal daily scores retain dated co-belligerents, missing-side names and colours', () => {
  const reply = timeline();
  reply.lastHour = 47; reply.days = [23, 47];
  reply.sides = [{ ...sides[0], reads: [
    [0, sides[0].name, sides[0].leader, null, 1, [], '10'],
    [24, sides[0].name, sides[0].leader, null, 2, [], '10'],
  ] }];
  reply.rows[0].cells = [[2, [[0, '1']]]];
  const views = timelineViews(reply, looks);
  assert.equal(views.viewAt(7000, 0)?.parts[0].side?.coBelligerents, 1);
  assert.equal(views.viewAt(7000, 1)?.parts[0].side?.coBelligerents, 2);
  reply.sides[0].reads![1][1] = 'Later unknown faction';
  reply.rows[0].cells = [[2, { scores: [], unknown: [0] }]];
  const missing = timelineViews(reply, looks);
  assert.equal(missing.viewAt(7000, 0)?.unknown[0].name, sides[0].name);
  assert.equal(missing.viewAt(7000, 1)?.unknown[0].name, 'Later unknown faction');
  reply.rows[0].cells = [[2, [[0, '1']]]];
  reply.sides[0].reads![1][1] = sides[0].name;
  reply.sides[0].reads![1][6] = null;
  const datedLooks = { ...looks, color: (side: AirSide) => side.colourFaction ? '#336699' : '#993366' };
  const coloured = timelineViews(reply, datedLooks);
  assert.equal(coloured.viewAt(7000, 1)?.parts[0].color, '#993366');
  assert.equal(coloured.rows[0].cells[1].parts[0].color, '#993366');
});

test('Name rows retain absent-object and unknown defaults without filling gaps', () => {
  const reply = moment();
  reply.regions = [];
  reply.fallback = [[0, 23, 'no-air-object'], [72, 167, { scores: [], unknownSet: 0 }]];
  const decoded = validateAirReply({ ...reply, unknownSets: [[0]] }) as AirMoment;
  assert.equal(momentViews(decoded, 12, looks, choices())[0].view.state, 'no-air-object');
  assert.equal(momentViews(decoded, 24, looks, choices())[0].view.state, 'gap');
  assert.equal(momentViews(decoded, 72, looks, choices())[0].view.unknown[0].key, sides[0].key);
  assert.equal(JSON.stringify(reply.fallback[1][2]), JSON.stringify({ scores: [], unknownSet: 0 }));
  decoded.fallback = [[0, 23, 'outside-theatres']];
  assert.equal(orderedRegions(momentViews(decoded, 12, looks, choices()), 'name').length, 0);
  decoded.regions = [{ id: 7000, name: 'Known empty region', active: [], runs: [[0, 23, null]], wings: [] }];
  decoded.fallback = [[0, 23, 'score-not-recorded']];
  assert.equal(momentViews(decoded, 12, looks)[0].view.state, 'inactive');
});

test('region orders show score or wings, and Name includes every content choice', () => {
  const rows = momentViews(moment(), 0, looks, choices());
  assert.deepEqual(orderedRegions(rows, 'active').map(row => row.region.id), [7000, 8888]);
  assert.equal(orderedRegions(rows, 'name').length, 3);
  assert.equal(orderedRegions(momentViews(moment(), 24, looks), 'active').length, 0);
  assert.equal(orderedRegions(momentViews(moment(), 72, looks), 'active').length, 1);
  const filtered = moment(); filtered.regions[0].runs = [[0, 167, 'outside-theatres']];
  assert.equal(orderedRegions(momentViews(filtered, 0, looks, choices()), 'name').some(row => row.region.id === 7000), false);
  assert.equal(airState(cellView('outside-theatres', sides, looks, sideOrder(sides), 0)), 'Outside selected air theatres');
  const history = timeline(); history.rows[0].cells = [[4, 'outside-theatres']];
  const geometry = timelineViews(history, looks).rows[0].cells[0];
  assert.deepEqual(geometry.parts, []);
  assert.equal(geometry.unknown, false);
  const nonleader = moment(); nonleader.regions.push({ id: 9900, name: 'Only nonleader scores', runs: [], wings: [], active: [span(6, 7)] });
  const shown = orderedRegions(momentViews(nonleader, 6, looks), 'active');
  assert.equal(shown.find(row => row.region.id === 9900)?.view.state, 'inactive');
  assert.equal(orderedRegions(momentViews(nonleader, 8, looks), 'active').some(row => row.region.id === 9900), false);
  assert.ok(momentReadHours(nonleader).includes(6));
  assert.ok(momentReadHours(nonleader).includes(8));
});

test('most contested keeps unknown and wing-only leads after known comparisons', () => {
  const reply = moment();
  reply.regions.push({ id: 7777, name: 'Unknown wing region', active: [span(0, 23)],
    runs: [[0, 23, { scores: [[0, '45']], unknown: [1] }]], wings: [[0, 23, 1]] });
  const rows = orderedRegions(momentViews(reply, 0, looks), 'contested');
  assert.equal(rows[0].region.id, 7000);
  assert.deepEqual(rows.slice(1).map(row => row.region.id), [7777, 8888]);
  assert.equal(rows[1].view.state, 'unknown');
  assert.equal(rows[2].view.state, 'inactive');
});

test('complete absent owners, unknown pairs and selected-side inactivity stay distinct', () => {
  const order = sideOrder(sides);
  const excluded = cellView({ scores: [[0, '45']], noAirObject: [1], excluded: 4997 }, sides, looks, order, 0);
  assert.equal(excluded.state, 'known');
  assert.ok(tooltipText(airTooltip({ name: 'Mod region' }, excluded, false, tag => ({ name: tag }), false)).includes('No air data'));
  assert.match(tooltipText(airTooltip({ name: 'Mod region' }, excluded, false, tag => ({ name: tag }), false)), /Excluded sides 4997/);
  const unknown = cellView({ scores: [[0, '45']], unknown: [1] }, sides, looks, order, 0);
  assert.equal(unknown.state, 'unknown');
  assert.match(tooltipText(airTooltip({ name: 'Mod region' }, unknown, false, tag => ({ name: tag }), false)), /Share unknown/);
  const unknownTip = airSummaryTooltip(airTooltip({ name: 'Mod region' }, unknown, false, tag => ({ name: tag }), false), unknown);
  assert.equal(unknownTip.rows.find(row => row.label === 'Unknown sides')?.value, '1');
  assert.equal(unknownTip.rows.some(row => row.value === 'Score unknown'), false);
  const filtered = cellView({ scores: [], filtered: true }, sides, looks, order, 0);
  assert.equal(airState(filtered), 'No selected side has a score (others may)');
  assert.equal(airState(cellView(null, sides, looks, order, 0)), 'No air activity');
  assert.equal(airState(cellView('no-air-object', sides, looks, order, 0)), 'No air data');
  const sideNoObject = cellView({ scores: [], noAirObject: [1] }, sides, { ...looks, mode: 'viewpoint' }, order, 0);
  assert.equal(sideNoObject.state, 'no-air-object');
  assert.equal(sideNoObject.viewpoint.tag, 'Other');
  assert.equal(cellView({ scores: [], noAirObject: [1] }, sides, looks, order, 0).state, 'inactive');
  const reply = moment(); reply.excluded = [[0, 11, 4997], [12, 23, 4996]];
  assert.equal(momentViews(reply, 0, looks)[0].view.excluded, 4997);
  assert.equal(momentViews(reply, 12, looks)[0].view.excluded, 4996);
  const history = timeline(); history.excluded = [[0, 23, 4997], [72, 73, 4996]];
  const views = timelineViews(history, looks);
  assert.equal(views.viewAt(7000, 0)?.excluded, 4997);
  assert.equal(views.viewAt(7000, 3)?.excluded, 4996);
});

test('one side key preserves dated context and regional coalition viewpoints', () => {
  const reply = timeline();
  reply.rows[0].cells[0][1] = [[0, '4500000'], [1, '3500000'], [2, '2000000']];
  assert.equal(validateAirReply(reply).view, 'timeline');
  const order = sideOrder(sides);
  assert.deepEqual(cellView(reply.rows[0].cells[0][1], sides, looks, order, 23).parts.map(part => part.id), ['f:9', 'f:10', 'w:third']);
  assert.equal(cellView([], sides, looks, order, 23).state, 'inactive');
  const variants = [...sides];
  variants[2] = { ...sides[2], reads: [[0, 'A coalition', null, null, 0, [], null], [72, 'Recorded later coalition', null, null, 0, [], null]] };
  const view = cellView({ view: ['75', '25'], side: 2, leader: 'Later regional leader' }, variants, looks, sideOrder(variants), 73);
  assert.equal(view.viewpoint.name, 'Recorded later coalition');
  assert.equal(view.viewpoint.tag, 'Later regional leader');
  assert.equal(view.parts[0].tag, 'Later regional leader');
  assert.equal(view.parts[0].side?.key, variants[2].key);
  assert.equal(cellView({ view: ['75', '25'], side: 2, leader: 'Earlier leader' }, variants, looks, sideOrder(variants), 23).viewpoint.name, 'A coalition');
  assert.match(tooltipText(airTooltip({ name: 'Mod region' }, view, true, tag => ({ name: tag }), false)), /Viewpoint Later regional leader/);
  reply.rows[0].cells[0][1] = { view: ['1', '2'], side: 99, leader: null };
  assert.throws(() => validateAirReply(reply), /Invalid air/);
});

test('tooltip ranks largest shares first and dates names, viewpoints and co-belligerents', () => {
  const view = cellView({ scores: [[1, '35'], [0, '45'], [2, '20']] }, sides, looks, sideOrder(sides), 72, 3);
  const tip = airTooltip({ name: 'Mod region' }, view, false, tag => ({ name: 'Name at read: ' + tag, flag: `read-${view.hour}:${tag}` }), true);
  assert.deepEqual(tip.rows.slice(0, 3).map(row => row.label), ['Mod tenth faction', 'Mod ninth faction', 'Name at read: Coalition leader']);
  assert.equal(tip.rows[2].countryFlag?.flag, 'read-72:Coalition leader');
  assert.match(tooltipText(tip), /with 3 co-belligerents/);
  assert.match(tip.note ?? '', /only the selected sides/);
  const compact = airSummaryTooltip(tip, view);
  assert.deepEqual(compact.rows.filter(row => row.swatch).map(row => row.value), ['45%', '35%', '20%']);
  const viewpoint = cellView(['45', '55'], sides, looks, sideOrder(sides), 72);
  const exact = airTooltip({ name: 'Mod region' }, viewpoint, true, () => ({ name: 'Historical name' }), false);
  assert.equal(exact.rows[0].label, 'Enemies of Historical name');
  assert.equal(exact.rows[1].label, 'Historical name (friendly)');
  assert.match(exact.note ?? '', /Above 60%.*below 40%/);
  assert.doesNotMatch(exact.note ?? '', /fixed5|64-bit|Exact viewpoint|friendly \//);
  assert.equal(exact.rows.some(row => /Read|membership read/.test(row.label)), false);
  for (const [friendly, state] of [[39, 'Enemy superiority'], [40, 'Contested'], [60, 'Contested'], [61, 'Air superiority']] as const) {
    assert.equal(airState(cellView([String(friendly), String(100 - friendly)], sides, looks, sideOrder(sides), 0)), state);
  }
  assert.equal(airState(cellView(['6000001', '3999999'], sides, looks, sideOrder(sides), 0)), 'Contested');
  assert.equal(airState(cellView(['3999999', '6000001'], sides, looks, sideOrder(sides), 0)), 'Enemy superiority');
  const wrapped = cellView(['9223372036854775805', '0'], sides, looks, sideOrder(sides), 0);
  assert.equal(airState(wrapped), 'Enemy superiority');
  assert.match(airTooltip({ name: 'Mod region' }, wrapped, true, tag => ({ name: tag }), false).note ?? '', /Game status differs from the displayed shares/);
  assert.match(airTooltip({ name: 'Mod region' }, wrapped, true, tag => ({ name: tag }), false).note ?? '', /Above 60%.*below 40%/);
  assert.equal(airState(cellView(['9223372036854775807', '9223372036854775807'], sides, looks, sideOrder(sides), 0)), 'Air superiority');
  assert.equal(airPercent(9223372036854775807n, 18446744073709551614n), '50%');
});

test('theatre filters date their daily membership separately from hourly scores', () => {
  const history = timeline(); history.theatreReads = [[23, 23, 6], [73, 73, 54]];
  const daily = timelineViews(validateAirReply(history) as AirTimeline, looks);
  assert.equal(daily.viewAt(7000, 0)?.theatreRead, 6);
  assert.equal(daily.viewAt(7000, 1)?.theatreRead, undefined);
  assert.equal(daily.viewAt(7000, 3)?.theatreRead, 54);
  const reply = moment(); reply.theatreReads = [[0, 23, -18], [72, 95, 54], [96, 119, 78]];
  const current = momentViews(validateAirReply(reply) as AirMoment, 96, looks)[0].view;
  assert.equal(current.hour, 96);
  assert.equal(current.theatreRead, 78);
  assert.ok(momentReadHours(reply).includes(96));
  assert.ok(momentReadHours(reply).includes(120));
  const tip = airTooltip({ name: 'Mod region' }, current, false, tag => ({ name: tag }), false);
  assert.ok(!tip.rows.some(row => row.label === 'Air theatre membership read'));
  assert.equal(airTooltip({ name: 'Mod region' }, momentViews(moment(), 96, looks)[0].view, false, tag => ({ name: tag }), false).rows.some(row => row.label === 'Air theatre membership read'), false);
});

test('scores read on the denominator the reply gives, never as that many decimals', () => {
  assert.equal(airScore(4500000n, 100000), '45');
  assert.equal(airScore(123456n, 100000), '1.23');
  assert.equal(airScore(0n, 100000), '0');
  const view = timelineViews(validateAirReply(timeline()) as AirTimeline, looks).viewAt(7000, 0)!;
  const text = tooltipText(airTooltip({ name: 'Mod region' }, view, false, tag => ({ name: tag }), false));
  assert.match(text, /45% · 45 score/);
  assert.match(text, /20% · 20 score/);
});

test('missing-data tooltips explain recorded omissions in plain words', () => {
  const reasons = new Map([
    ['score-not-recorded', 'Scores were not recorded at this hour.'],
    ['scores-unavailable', 'Scores were not recorded at this hour.'],
    ['sides-not-recorded', 'Side membership was not recorded.'],
    ['countries-not-recorded', 'Countries unavailable.'],
    ['identity-not-recorded', 'Countries unavailable.'],
    ['theatres-not-recorded', 'Air groups unavailable.'],
    ['future-missing-reason', 'The required data was not recorded.'],
  ]);
  for (const [reason, description] of reasons) {
    const view = cellView(reason, sides, looks, sideOrder(sides), 72);
    const tip = airTooltip({ name: 'Mod region' }, view, false, tag => ({ name: tag }), false);
    assert.ok(tip.note?.includes(description));
    assert.equal(tip.note?.includes(reason), false);
  }
});

test('malformed RLE, indexes, raw scores, ranges and choices refuse the reply', () => {
  for (const mutate of [
    (reply: AirTimeline) => { reply.rows[0].cells[0][0] = 8; },
    (reply: AirTimeline) => { reply.rows[0].cells[0][1] = { scores: [[50, '1']] }; },
    (reply: AirTimeline) => { reply.rows[0].cells[0][1] = ['-1', '0']; },
    (reply: AirTimeline) => { reply.rows[0].cells[0][1] = { scores: [[0, '1']], unknown: [0] }; },
    (reply: AirTimeline) => { reply.days[0] = 48; },
    (reply: AirTimeline) => { reply.rows[0].cells[0][1] = { scores: [], excluded: -1 }; },
  ]) { const reply = timeline(); mutate(reply); assert.throws(() => validateAirReply(reply), /Invalid air/); }
  const badMoment = moment(); badMoment.regions[0].runs[1][0] = 11;
  assert.throws(() => validateAirReply(badMoment), /Invalid air/);
  const badChoices = choices(); badChoices.theatres[0].name = 7 as unknown as string;
  assert.throws(() => validateAirReply(badChoices), /Invalid air/);
  const badWindows = choices(); badWindows.windows[1].firstHour = 4999;
  assert.throws(() => validateAirReply(badWindows), /Invalid air/);
  const missingCountries = { ...choices(), countries: undefined };
  const missingExcluded = { ...moment(), excluded: undefined };
  const missingTheatreReads = { ...moment(), theatreReads: undefined };
  const futureTheatreRead = moment(); futureTheatreRead.theatreReads = [[0, 23, 1]];
  const overlappingTheatreRead = moment(); overlappingTheatreRead.theatreReads = [[0, 12, 0], [12, 23, 0]];
  const missingActive = moment();
  missingActive.regions[0].active = undefined as unknown as AirMoment['regions'][number]['active'];
  const badActive = moment(); badActive.regions[0].active = [span(7, 8), span(8, 9)];
  for (const reply of [missingCountries, missingExcluded, missingTheatreReads, futureTheatreRead, overlappingTheatreRead, missingActive, badActive]) assert.throws(() => validateAirReply(reply), /Invalid air/);
  assert.equal(validateAirReply(choices()).view, 'choices');
});

test('varying projections retain every score through 5,000 independent mod sides', () => {
  for (const count of [1500, 5000]) {
    const reply = timeline();
    reply.lastHour = 71; reply.days = [23, 47, 71];
    reply.sides = Array.from({ length: count }, (_, index) => ({ key: 'c:mod ' + index, faction: null, colourFaction: null, tag: 'mod ' + index, name: 'Mod ' + index, leader: 'mod ' + index, coBelligerents: 0 }));
    reply.rows = Array.from({ length: 2 }, (_, region) => ({ region: 7000 + region, name: 'Mod region ' + region,
      cells: Array.from({ length: 3 }, (_, day) => [1, { scores: reply.sides.map((_, side) => [side, String((side + day * 7 + region * 11) % 101 + 1)]) }] as AirTimeline['rows'][number]['cells'][number]) }));
    const projected = timelineViews(reply, looks);
    for (const row of projected.rows) for (const cell of row.cells) assert.equal(cell.parts.length, count);
    assert.equal(projected.rows[0].cells[0].parts.at(-1)?.id, `c:mod ${count - 1}`);
    assert.notDeepEqual(projected.rows[0].cells[0].parts[0].share, projected.rows[0].cells[1].parts[0].share);
  }
});

test('faction and theatre choices belong to their campaign and follow requests are day-granular', () => {
  for (const key of ['f:7', 'w:an ally', 'opaque theatre']) {
    const stored = airChoiceToken('campaign-a', key);
    assert.equal(airChoiceKey('campaign-a', stored), key);
    assert.equal(airChoiceKey('campaign-b', stored), null);
  }
  assert.equal(timelineRequestLast(25, 24), timelineRequestLast(46, 24));
  assert.notEqual(timelineRequestLast(47, 24), timelineRequestLast(48, 24));
  assert.equal(timelineRequestLast(-1, 24), -24);
});

test('the current partial day retains its last reached read across an intra-day gap', () => {
  const recorded = [span(24, 31), span(41, 47)];
  assert.equal(lastRecordedAt(recorded, 23), null);
  assert.equal(lastRecordedAt(recorded, 35), 31);
  assert.equal(lastRecordedAt(recorded, 43), 43);
});

test('recorded country looks include every regional coalition viewpoint', () => {
  const history = timeline();
  history.rows[0].cells = [[4, { scores: [[2, '20']], viewpoints: [[2, 'regional winner']] }]];
  assert.ok(airCountryTags(history).includes('regional winner'));
  const window = moment();
  window.regions[0].runs = [[0, 23, { view: ['20', '10'], side: 2, leader: 'other regional winner' }]];
  assert.ok(airCountryTags(window).includes('other regional winner'));
});

test('Follow retains both dated replies only within its client, recording and comparison', () => {
  const client = {} as DashboardClient;
  const held = { client, identity: 'same recording and filters', timeline: timeline(), day: moment() };
  assert.equal(heldAirReply(held, client, held.identity, 'timeline'), held.timeline);
  assert.equal(heldAirReply(held, client, held.identity, 'day'), held.day);
  assert.equal(heldAirReply(held, {} as DashboardClient, held.identity, 'timeline'), undefined);
  assert.equal(heldAirReply(held, client, 'different comparison', 'timeline'), undefined);
  assert.equal(heldAirReply(undefined, client, held.identity, 'day'), undefined);
});

test('coalition parts keep one look while regional viewpoints change', () => {
  const viewLooks = { ...looks, name: (side: typeof sides[number]) => side.leader ?? side.name, color: (side: typeof sides[number]) => side.leader === 'regional winner' ? '#ff0000' : '#0000ff' };
  const view = cellView({ scores: [[2, '25']], viewpoints: [[2, 'regional winner']] }, sides, viewLooks, sideOrder(sides), 23);
  assert.equal(view.parts[0].side?.leader, sides[2].leader);
  assert.equal(view.parts[0].color, '#0000ff');
  assert.equal(view.parts[0].tag, 'regional winner');
  const tip = airSummaryTooltip(airTooltip({ name: 'Mod region' }, view, false, tag => ({ name: tag }), false), view);
  assert.ok(!tip.rows.some(row => row.label === 'Uncontested'));
});


test('coalition names use dated present members while their colour identity stays historical', () => {
  const remnant = { key: 'w:CTL', faction: null, colourFaction: null, tag: 'MOD', name: 'MOD', leader: null, coBelligerents: 0 };
  assert.equal(airSideTag(remnant), 'MOD');
  assert.equal(airSideName(remnant, tag => `Name ${tag}`), 'Name MOD');
  assert.equal(airSideName({ ...remnant, coBelligerents: 2 }, tag => `Name ${tag}`), 'Name MOD and allies');
});

test('historical share labels and coalition viewpoints ask for every named actor', () => {
  const coalition = { key: 'w:anchor', faction: null, colourFaction: null, tag: 'first member', name: 'first member', leader: null, coBelligerents: 1 };
  const view = cellView({ scores: [[0, '50']], viewpoints: [[0, 'regional winner']] }, [coalition],
    { ...looks, name: () => 'Latest member name' }, sideOrder([coalition]), 23);
  assert.ok(airTooltipTags(view).includes('first member'));
  assert.ok(airTooltipTags(view).includes('regional winner'));
  const tip = airTooltip({ name: 'Region' }, view, false, tag => ({ name: `Earlier ${tag}` }), false);
  assert.equal(tip.rows.find(row => row.swatch)?.label, 'Earlier first member and allies');
  assert.match(tooltipText(tip), /Viewpoint: Earlier regional winner/);
  assert.ok(!tooltipText(tip).includes('Latest member name'));
  const exact = cellView({ side: 0, view: ['50', '20'], leader: null }, [coalition],
    { ...looks, name: () => 'Latest member name' }, sideOrder([coalition]), 23);
  const exactTip = airTooltip({ name: 'Region' }, exact, true, tag => ({ name: `Earlier ${tag}` }), false);
  assert.ok(exactTip.rows.some(row => row.label === 'Enemies of Earlier first member and allies'));
});

test('native country identity bounds do not carry looks through tag reuse or absence', () => {
  const segments = { opaque: [span(0, 5), span(8, 10), span(11, 15)] };
  assert.equal(airSegmentAt(segments, 'opaque', 5), 0);
  assert.equal(airSegmentAt(segments, 'opaque', 6), null);
  assert.equal(airSegmentAt(segments, 'opaque', 10), 8);
  assert.equal(airSegmentAt(segments, 'opaque', 11), 11);
  const reply = { ...moment(), firstHour: 72, segments: { anchor: [span(0, 5)], current: [span(70, 167)] }, anchorReads: { 'w:third': ['anchor', 2] } };
  // These empty rows isolate look metadata from the moment's score runs.
  reply.regions = []; reply.recorded = []; reply.complete = []; reply.wingsKnown = [];
  assert.equal(validateAirReply(reply).view, 'moment');
  assert.throws(() => validateAirReply({ ...reply, anchorReads: { 'w:third': ['anchor', 6] } }), /Invalid air/);
  assert.throws(() => validateAirReply({ ...reply, segments: { current: [span(0, 71), span(71, 167)] } }), /Invalid air/);
});


test('moment wire shares unknown rosters and expands compact countries without losing dates', () => {
  const countryCount = 5000;
  const compactSides = Array.from({ length: countryCount }, (_, index) => [`mod ${index}`, 72]);
  const unknown = compactSides.map((_, index) => index);
  const raw = { ...moment(), sides: compactSides, unknownSets: [unknown],
    regions: [7000, 7001].map(id => ({ id, name: `Region ${id}`, active: [span(72, 95)],
      runs: [[72, 95, { scores: [], unknownSet: 0 }]], wings: [] })) };
  const before = JSON.stringify(raw);
  const reply = validateAirReply(raw) as AirMoment;
  assert.equal(JSON.stringify(raw), before, 'normalisation must not mutate a cached wire reply');
  assert.equal(reply.sides.length, countryCount);
  assert.deepEqual(reply.sides.at(-1), { key: 'c:mod 4999', faction: null, colourFaction: null, tag: 'mod 4999', name: 'Country name unavailable', leader: 'mod 4999', coBelligerents: 0,
    reads: [[72, 'Country name unavailable', 'mod 4999', 'mod 4999', 0, [], null]] });
  assert.deepEqual(reply.regions[0].runs[0][2], { scores: [], unknown });
  assert.equal(momentViews(reply, 73, looks)[0].view.unknown.length, countryCount);
  assert.equal(momentViews(reply, 73, looks)[1].view.unknown.at(-1)?.key, 'c:mod 4999');
  assert.deepEqual(validateAirReply(reply), reply);
});

test('moment uncertainty references reject stale, overlapping and out-of-range side lists', () => {
  const raw = { ...moment(), unknownSets: [[1, 2]] };
  raw.regions[0].runs = [[0, 23, { scores: [[0, '1']], unknownSet: 0 }]];
  assert.equal(validateAirReply(raw).view, 'moment');
  for (const mutate of [
    (reply: typeof raw) => { reply.unknownSets = [[0]]; },
    (reply: typeof raw) => { reply.unknownSets = [[99]]; },
    (reply: typeof raw) => { reply.unknownSets = [[1, 1]]; },
    (reply: typeof raw) => { reply.regions[0].runs[0][2] = { scores: [], unknownSet: 9 }; },
    (reply: typeof raw) => { reply.regions[0].runs[0][2] = { scores: [], unknownSet: 0, unknown: [1] }; },
  ]) {
    const reply = structuredClone(raw); mutate(reply);
    assert.throws(() => validateAirReply(reply), /Invalid air/);
  }
  const timelineReply = timeline(); timelineReply.rows[0].cells = [[4, { scores: [], unknownSet: 0 }]];
  assert.throws(() => validateAirReply({ ...timelineReply, unknownSets: [[0]] }), /Invalid air/);
  for (const side of [['mod', '72'], ['mod'], ['', 72]]) {
    assert.throws(() => validateAirReply({ ...moment(), sides: [side] }), /Invalid air/);
  }
});

test('air tooltip forwards a native leader flag state while a side with no leader stays flagless', () => {
  const observed = [{ ...sides[0], leader: null }, sides[1]];
  const view = cellView({ scores: [[0, '45'], [1, '35']] }, observed, looks, sideOrder(observed), 42);
  const label = { name: 'Other at 42', flagReason: 'Native flag definition absent at 42', flagPending: false };
  const tip = airTooltip({ name: 'Mod region' }, view, false, tag => tag === 'Other' ? label : { name: tag }, false);
  const rows = tip.rows.filter(row => row.swatch);
  assert.equal(rows.length, 2);
  assert.equal(rows.filter(row => row.countryFlag).length, 1);
  assert.deepEqual(rows.find(row => row.countryFlag)?.countryFlag, label);
  assert.ok(rows.every(row => row.icon === undefined));
});


test('timeline projection gives each equal-score country cell its own colour date', () => {
  const reply = { ...timeline(), lastHour: 47, days: [23, 47], rows: [{ region: 7000, name: 'Synthetic', cells: [[2, { scores: [[0, '1']] }]] as AirTimeline['rows'][number]['cells'] }] };
  const projected = timelineViews(reply, { ...looks, color: (_, hour) => hour === 23 ? '#102030' : '#708090' });
  assert.equal(projected.rows[0].cells[0].parts[0].color, '#102030');
  assert.equal(projected.rows[0].cells[1].parts[0].color, '#708090');
});

test('timeline days alike in looks and sides take the day before’s geometry without being projected again', () => {
  const days = Array.from({ length: 300 }, (_, day) => day * 24 + 23);
  const reply: AirTimeline = { ...timeline(), lastHour: days.at(-1)!, days, rows: [{ region: 7000, name: 'Long mod region', cells: [[300, { scores: [[0, '3'], [1, '1']] }]] }] };
  const look = (hour: number) => hour < days[150] ? 'early' : 'late';
  let asked = 0;
  const projected = timelineViews(reply, { ...looks, lookAt: look, color: (_, hour) => { asked++; return look(hour!) === 'early' ? '#336699' : '#993366'; } });
  assert.equal(asked, 4, 'the run and the day its looks change');
  const cells = projected.rows[0].cells;
  assert.equal(cells[149], cells[0]);
  assert.equal(cells[150].parts[0].color, '#993366');
  assert.deepEqual(cells[299], cells[150]);
  assert.notEqual(cells[299].parts, cells[150].parts, 'each differing day keeps its own stripe');
  assert.equal(projected.viewAt(7000, 299)?.parts[0].color, '#993366');
});

test('sampled daily cells retain their real supplying hour and approximation markers', () => {
  const reply = timeline();
  reply.sampleHours = [10, null, null, 60];
  reply.approximate = [span(0, 23), span(72, 73)];
  reply.pending = [span(24, 71)];
  reply.coverage = { firstHour: 0, lastHour: -1, complete: false };
  reply.rowsApproximate = true;
  const valid = validateAirReply(reply) as AirTimeline;
  assert.deepEqual(valid.approximate, reply.approximate);
  assert.equal(airPeriods(valid)[0].readHour, 10);
  assert.equal(timelineViews(valid, looks).viewAt(7000, 0)?.hour, 10);
  assert.throws(() => validateAirReply({ ...reply, approximate: [span(9, 8)] }), /Invalid air|malformed/);
});
