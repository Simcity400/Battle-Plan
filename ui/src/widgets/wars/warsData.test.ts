import assert from 'node:assert/strict';
import test from 'node:test';
import { Lineage } from '../../dashboard/identity.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import type { WarObservation, WarParticipantRun, WarTimeline, WarsReply } from '../../generated/dashboard';
import { countryTagAt, lossesOf, structureAt, structureHours, validateWarContributions, validateWarLosses, isWarsReply, joinedAt, laneCountries, observationAt, labelObservationAt, participantHoldingAt, prepareWars, retainExpandedWars, sideLabel, validateWarsReply, viewpointAt, warNamesAt, warsLanes } from './warsData.ts';
import type { WarsLaneOptions } from './warsData';

const participant = (tag: string, side: WarParticipantRun['side'], firstHour = 10, lastHour = 19): WarParticipantRun =>
  ({ tag, side, firstHour, lastHour, joinedBy: true, leftBy: false });
const observation = (firstHour = 10, lastHour = 19, attackersAreViewpoint: boolean | null = true): WarObservation => ({
  firstHour, lastHour, viewpointTag: 'Mod: player', viewpointSide: ['One side'], opposingSide: ['Other side'],
  attackersAreViewpoint, representative: ['One side', 'Other side'], name: { text: 'Recorded name', known: true, reason: null, key: null },
});
const war = (id = '1', firstHour = 10, lastHour = 19): WarTimeline => ({
  id, firstHour, lastHour, startedBy: true, endedBy: false, observations: [observation(firstHour, lastHour)],
  participants: [participant('One side', 'attacker', firstHour, lastHour), participant('Other side', 'defender', firstHour, lastHour)],
  capitulations: [],
});
const reply = (wars: WarTimeline[] = [war()]): WarsReply => ({ countryNames: [], branchId: 'branch', available: true, viewpoints: [], wars, unobserved: [], capitulationsUnobserved: [], warnings: [] });
const options: WarsLaneOptions = {
  view: 'gantt', expanded: new Set(), colour: 'war', palette: ['first', 'second', 'third'], named: new Set(['One side', 'Other side', 'Mod: player']), neutral: 'neutral',
  country: tag => ({ name: `Name ${tag}`, known: true, flag: `flag:${tag}`, color: `map:${tag}` }),
};

test('native replacements split participant lanes without a rename event and never inherit old labels', () => {
  const identities = countryIdentityIndex([
    { tag: 'One side', identities: [
      { firstHour: 10, lastHour: 14, slot: 1, countryId: 101 },
      { firstHour: 15, lastHour: 19, slot: 1, countryId: 201 },
    ] },
    { tag: 'Other side', identities: [{ firstHour: 10, lastHour: 19, slot: 2, countryId: 102 }] },
  ]);
  const prepared = prepareWars(reply(), new Lineage([]), identities);
  const members = prepared.wars[0].participants.filter(member => member.tag === 'One side');
  assert.equal(members.length, 2);
  assert.deepEqual(members.map(member => member.runs.map(run => [run.firstHour, run.lastHour])), [[[10, 14]], [[15, 19]]]);
  assert.equal(countryTagAt(members[0], 17), null);
  assert.equal(countryTagAt(members[1], 17), 'One side');
  const shown = warsLanes(prepared, 17, { ...options, view: 'countries' });
  const oldLane = shown.lanes.find(lane => {
    const info = shown.info.get(lane.id);
    return info?.kind === 'country' && info.country.key === members[0].key;
  });
  assert.equal(oldLane?.identity?.label, 'Not recorded');
  assert.deepEqual(laneCountries(shown.lanes, shown.info, 17).sort(), ['One side', 'Other side']);
});

test('native aliases keep one lane and missing holder hours never request a country look', () => {
  const item = war();
  item.participants = [participant('Before', 'attacker', 10, 14), participant('After', 'attacker', 15, 19)];
  const identities = countryIdentityIndex([
    { tag: 'Before', identities: [{ firstHour: 10, lastHour: 14, slot: 1, countryId: 101 }] },
    { tag: 'After', identities: [{ firstHour: 15, lastHour: 19, slot: 1, countryId: 101 }] },
  ]);
  const prepared = prepareWars(reply([item]), new Lineage([]), identities);
  assert.equal(prepared.countries.length, 1);
  assert.equal(countryTagAt(prepared.countries[0], 17), 'After');
  assert.equal(countryTagAt(prepared.countries[0], 20), null);
  const shown = warsLanes(prepared, 20, { ...options, view: 'countries', country: () => { throw new Error('Missing native holder requested a look'); } });
  assert.equal(shown.lanes[0].identity?.label, 'Not recorded');
  assert.deepEqual(laneCountries(shown.lanes, shown.info, 20), []);
});

test('a renamed member joined when its native country did, and a first sighting is only an upper bound', () => {
  const item = war('1', 10, 30);
  item.participants = [{ ...participant('Before', 'attacker', 12, 19), joinedBy: false }, { ...participant('After', 'attacker', 20, 30), joinedBy: false },
    participant('Late', 'defender', 25, 30)];
  const identities = countryIdentityIndex([
    { tag: 'Before', identities: [{ firstHour: 10, lastHour: 19, slot: 1, countryId: 101 }] },
    { tag: 'After', identities: [{ firstHour: 20, lastHour: 30, slot: 1, countryId: 101 }] },
    { tag: 'Late', identities: [{ firstHour: 10, lastHour: 30, slot: 2, countryId: 102 }] },
  ]);
  assert.deepEqual(joinedAt(item, 'After', 28, identities), { firstHour: 12, joinedBy: false });
  assert.deepEqual(joinedAt(item, 'Late', 28, identities), { firstHour: 25, joinedBy: true });
  assert.equal(joinedAt(item, 'Late', 22, identities), null);
  assert.equal(joinedAt(item, 'Unknown', 28, identities), null);
});

test('validate the complete dated contract, including availability and arbitrary exact signed casualty strings', () => {
  const data = reply();
  assert.equal(validateWarsReply(data), data);
  for (const bad of [null, {}, { ...data, available: undefined }, { ...data, countryNames: undefined }, { ...data, viewpoints: undefined }, { ...data, warnings: [null] }, { ...data, wars: [data.wars[0], data.wars[0]] },
    { ...data, wars: [{ ...war(), participants: [{ ...participant('A', 'attacker'), side: 'invented' }] }] },
    { ...data, unobserved: [{ firstHour: 14, lastHour: 14 }] }]) {
    assert.equal(isWarsReply(bad), false);
    assert.throws(() => validateWarsReply(bad), /malformed/);
  }
  assert.equal(isWarsReply(reply([])), true);
  assert.equal(isWarsReply({ ...reply([]), available: false }), true);
});

test('the observed viewpoint follows peacetime player changes and stays unknown across missing hours', () => {
  const data = reply([]);
  data.viewpoints = [{ tag: 'First player', firstHour: 0, lastHour: 9 }, { tag: 'Second player', firstHour: 10, lastHour: 19 },
    { tag: 'Second player', firstHour: 30, lastHour: 39 }];
  assert.equal(validateWarsReply(data), data);
  assert.equal(viewpointAt(data, -1), null);
  assert.equal(viewpointAt(data, 9.9), 'First player');
  assert.equal(viewpointAt(data, 10), 'Second player');
  assert.equal(viewpointAt(data, 25), null);
  assert.equal(viewpointAt(data, 40), null);
  assert.equal(isWarsReply({ ...data, unobserved: [{ firstHour: 15, lastHour: 15 }] }), false);
});

test('covering observations do not carry a name or roster through a gap', () => {
  const item = war('gap', 10, 39);
  item.observations = [observation(10, 19), { ...observation(30, 39), name: { text: 'Second name', known: true, reason: null, key: null } }];
  item.participants = [participant('One side', 'attacker', 10, 19), participant('One side', 'attacker', 30, 39)];
  const data = reply([item]);
  data.unobserved = [{ firstHour: 20, lastHour: 29 }];
  validateWarsReply(data);
  assert.equal(observationAt(item, 19.9)?.name.text, 'Recorded name');
  assert.equal(observationAt(item, 25), null);
  const prepared = prepareWars(data);
  assert.equal(participantHoldingAt(prepared.wars[0].participants[0], 25), null);
  const shown = warsLanes(prepared, 39, options);
  assert.deepEqual(shown.lanes[0].spans.map(span => [span.firstHour, span.lastHour]), [[10, 19], [30, 39]]);
  const inGap = warsLanes(prepared, 25, options).lanes[0];
  assert.equal(inGap.identity?.label, 'Not recorded');
  assert.equal(inGap.muted, true);
  assert.equal(warsLanes(prepared, 45, options).lanes[0].identity?.label, 'Second name');
});

test('unknown recorded names stay muted and bounded starts and joins remain the product values', () => {
  const item = war();
  item.observations[0].name = { text: 'Not recorded', known: false, reason: 'The naming operand was unavailable.', key: null };
  item.endedBy = true;
  item.participants[0].leftBy = true;
  const data = prepareWars(reply([item]));
  const shown = warsLanes(data, 15, { ...options, expanded: new Set([item.id]) });
  assert.equal(shown.lanes[0].identity?.label, 'Not recorded');
  assert.equal(shown.lanes[0].muted, true);
  assert.equal(data.wars[0].war.startedBy, true);
  assert.equal(data.wars[0].war.endedBy, true);
  assert.equal(data.wars[0].participants[0].runs[0].joinedBy, true);
  assert.equal(data.wars[0].participants[0].runs[0].leftBy, true);
});

test('expansion gives native sides the observed orientation and map-coloured country runs', () => {
  const item = war();
  item.observations[0].attackersAreViewpoint = false;
  item.participants = [participant('One side', 'defender'), participant('Other side', 'attacker')];
  const data = prepareWars(reply([item]));
  assert.equal(warsLanes(data, 9, options).lanes.length, 0);
  assert.equal(warsLanes(data, 15, options).lanes.length, 1);
  const shown = warsLanes(data, 15, { ...options, expanded: new Set([item.id]) });
  assert.deepEqual(shown.lanes.map(lane => lane.identity?.label), ['Recorded name', 'Attackers', 'Name Other side', 'Defenders', 'Name One side']);
  assert.deepEqual(shown.lanes.map(lane => lane.depth), [undefined, 1, 1, 1, 1]);
  assert.equal(shown.lanes[0].spans[0].color, 'neutral');
  assert.equal(shown.lanes[2].spans[0].color, 'map:Other side');
  assert.equal(shown.lanes[2].spans[0].openEnd, true);
  assert.deepEqual(laneCountries(shown.lanes.slice(1, 3), shown.info, 15), ['Other side']);
});

test('missing orientation keeps native side headings without inventing attackers or defenders', () => {
  const item = war();
  item.observations[0].attackersAreViewpoint = null;
  item.participants = [participant('One side', 'viewpoint'), participant('Other side', 'opponent')];
  const shown = warsLanes(prepareWars(reply([item])), 15, { ...options, expanded: new Set([item.id]) });
  assert.deepEqual(shown.lanes.filter(lane => lane.heading && lane.depth).map(lane => lane.identity?.label), ['Name Mod: player’s side', 'Opposing side']);
  assert.deepEqual(laneCountries(shown.lanes.slice(1, 2), shown.info, 15), ['Mod: player']);
  assert.equal(sideLabel('viewpoint', null, options.country), 'Viewpoint’s side');
  const country = warsLanes(prepareWars(reply([item])), 15, { ...options, view: 'countries', colour: 'side' });
  assert.ok(country.lanes.every(lane => lane.spans.every(span => span.color === 'neutral')));
});

test('viewpoint headings use read status, including native names equal to UI wording', () => {
  const unknown = observation(10, 19, null);
  for (const name of ['Loading…', 'Not recorded', '', 'Named country']) {
    assert.equal(sideLabel('viewpoint', unknown, () => ({ name, known: true })), `${name}’s side`);
    for (const known of [undefined, false]) assert.equal(sideLabel('viewpoint', unknown, () => ({ name, known })), 'Viewpoint’s side');
  }
});

test('internal gaps do not carry an earlier viewpoint into headings or name requests', () => {
  const item = war('gap', 10, 39);
  item.observations = [observation(10, 19), { ...observation(30, 39), viewpointTag: 'New player' }];
  item.participants = [participant('One side', 'attacker', 10, 19), participant('Other side', 'defender', 30, 39)];
  const shown = warsLanes(prepareWars(reply([item])), 25, { ...options, expanded: new Set([item.id]) });
  const sides = shown.lanes.filter(lane => lane.labelStyle === 'side');
  assert.deepEqual(sides.map(lane => lane.identity?.label), ['Viewpoint’s side', 'Opposing side']);
  assert.deepEqual(laneCountries(sides, shown.info, 25), []);
});

test('every capitulation keeps its recorded hour and is attributed only inside that participant run', () => {
  const item = war('events', 10, 39);
  item.capitulations = [{ tag: 'One side', hour: 16 }, { tag: 'One side', hour: 31 }];
  assert.equal(isWarsReply(reply([{ ...item, capitulations: [{ tag: 'Missing tag', hour: 20 }] }])), false);
  assert.equal(isWarsReply(reply([{ ...item, capitulations: [{ tag: 'One side', hour: 40 }] }])), false);
  const data = prepareWars(reply([item]));
  const shown = warsLanes(data, 39, { ...options, expanded: new Set([item.id]) });
  const lane = shown.lanes.find(lane => shown.info.get(lane.id)?.kind === 'participant' && lane.identity?.label === 'Name One side')!;
  assert.deepEqual(lane.spans.map(span => [span.firstHour, span.lastHour, span.glyphHours]), [[10, 39, [16, 31]]]);
  assert.equal(lane.spans[0].glyphShape, 'cross');
  assert.equal(lane.spans[0].glyph, undefined);
  assert.deepEqual([...shown.info.get(lane.id)!.spansById.values()].map(piece => piece.capitulations), [[16, 31]]);
});

test('country overlaps have fixed war order and disconnected participation reuses one slot', () => {
  const late = war('late', 20, 39);
  const early = war('early', 10, 29);
  const disconnected = war('after', 50, 59);
  for (const item of [late, early, disconnected]) item.participants = [participant('One side', 'attacker', item.firstHour, item.lastHour)];
  const data = prepareWars(reply([late, disconnected, early]));
  assert.deepEqual(data.wars.map(view => view.war.id), ['early', 'late', 'after']);
  const country = data.countries[0];
  assert.equal(country.subrows, 2);
  assert.deepEqual(country.spans.map(span => [span.war.id, span.subrow]), [['early', 0], ['late', 1], ['after', 0]]);
  const a = warsLanes(data, 25, { ...options, view: 'countries' });
  const b = warsLanes(data, 59, { ...options, view: 'countries' });
  assert.equal(a.lanes[0].subrows, b.lanes[0].subrows);
  assert.deepEqual(a.lanes[0].spans.map(span => [span.color, span.subrow]), [['first', 0], ['second', 1]]);
  assert.deepEqual(b.lanes[0].spans.map(span => [span.color, span.subrow]), [['first', 0], ['second', 1], ['third', 0]]);
  const sideColours = warsLanes(data, 59, { ...options, view: 'countries', colour: 'side' });
  assert.ok(sideColours.lanes[0].spans.every(span => span.color === 'first'));
});

test('country facts are requested only for named visible tags, once per tag', () => {
  const item = war();
  const requested: string[] = [];
  const shown = warsLanes(prepareWars(reply([item])), 15, { ...options, expanded: new Set([item.id]), named: new Set(['Other side']),
    country: tag => { requested.push(tag); return options.country(tag); } });
  assert.deepEqual(requested, ['Other side']);
  assert.equal(shown.lanes.find(lane => shown.info.get(lane.id)?.kind === 'participant' && lane.identity?.label === 'Loading…')?.identity?.countryFlag?.flagPending, true);
  assert.equal(shown.lanes.find(lane => lane.identity?.label === 'Name Other side')?.identity?.countryFlag?.flag, 'flag:Other side');
});

test('country lanes retain native loaded, pending and refused flag payloads without flagging war or side headings', () => {
  const prepared = prepareWars(reply());
  for (const flag of [{ flag: 'dated-flag.png' }, { flagPending: true },
    { flagPending: false, flagReason: 'The native custom flag is unavailable.' }]) {
    const look = { name: 'Dated country', known: true, ...flag };
    for (const view of ['gantt', 'countries'] as const) {
      const shown = warsLanes(prepared, 15, { ...options, view, expanded: new Set(['1']), country: () => look });
      for (const lane of shown.lanes) {
        const kind = shown.info.get(lane.id)?.kind;
        if (kind === 'country' || kind === 'participant') {
          assert.equal(lane.identity?.countryFlag, look);
          assert.equal(lane.identity?.icon, undefined);
        } else {
          assert.equal(lane.identity?.countryFlag, undefined);
        }
      }
    }
  }
});

test('opaque country tags and war ids never collide in lane or bar identities', () => {
  const items = [war('a:b'), war('a')];
  items[0].participants = [participant('c', 'attacker')];
  items[1].participants = [participant('b:c', 'attacker')];
  const data = prepareWars(reply(items));
  const shown = warsLanes(data, 15, { ...options, expanded: new Set(items.map(item => item.id)) });
  assert.equal(new Set(shown.lanes.map(lane => lane.id)).size, shown.lanes.length);
  const bars = shown.lanes.flatMap(lane => lane.spans.map(span => span.id));
  assert.equal(new Set(bars).size, bars.length);
});

test('renamed participants retain one country lane and names follow later changes after a war ends', () => {
  const item = war('renamed', 10, 19);
  item.observations = [{ ...observation(10, 14), viewpointSide: ['Before'] }, { ...observation(15, 19), viewpointSide: ['After'] }];
  item.participants = [participant('Before', 'attacker', 10, 14), participant('After', 'attacker', 15, 19)];
  const lineage = new Lineage([{ hour: 15, from: 'Before', to: 'After' }, { hour: 25, from: 'After', to: 'Later' }]);
  const data = prepareWars(reply([item]), lineage);
  assert.equal(data.countries.length, 1);
  assert.equal(data.wars[0].participants.length, 1);
  const member = data.wars[0].participants[0];
  assert.equal(countryTagAt(member, 12), 'Before');
  assert.equal(countryTagAt(member, 17), 'After');
  assert.equal(countryTagAt(member, 30), 'Later');
  assert.deepEqual(member.holdings.map(run => [run.firstHour, run.lastHour]), [[10, 19]]);
  assert.equal(participantHoldingAt(member, 12), participantHoldingAt(member, 17));
  assert.deepEqual(member.pieces.map(piece => [piece.run!.tag, piece.holding!.firstHour, piece.holding!.lastHour]),
    [['Before', 10, 19], ['After', 10, 19]]);
  const shown = warsLanes(data, 30, { ...options, expanded: new Set([item.id]), named: new Set(['Later']) });
  const participantLanes = shown.lanes.filter(lane => shown.info.get(lane.id)?.kind === 'participant');
  assert.equal(participantLanes.length, 1);
  assert.equal(participantLanes[0].identity?.label, 'Name Later');
  assert.deepEqual(laneCountries(participantLanes, shown.info, 12), ['Before']);
  assert.deepEqual(laneCountries(participantLanes, shown.info, 30), ['Later']);
  assert.deepEqual([...shown.info.get(participantLanes[0].id)!.spansById.values()].map(piece => piece.run?.tag), ['Before', 'After']);
});

test('holding bounds follow a country through a rename but remain separate at side changes and gaps', () => {
  const item = war('side-changed', 10, 19);
  item.participants = [participant('Before', 'attacker', 10, 14), participant('After', 'defender', 15, 19)];
  const lineage = new Lineage([{ hour: 15, from: 'Before', to: 'After' }]);
  const sideChange = prepareWars(reply([item]), lineage).wars[0].participants[0];
  assert.deepEqual(sideChange.holdings.map(run => [run.side, run.firstHour, run.lastHour]), [['attacker', 10, 14], ['defender', 15, 19]]);
  item.participants = [participant('Before', 'attacker', 10, 14), participant('After', 'attacker', 16, 19)];
  const gap = prepareWars(reply([item]), lineage).wars[0].participants[0];
  assert.deepEqual(gap.holdings.map(run => [run.firstHour, run.lastHour]), [[10, 14], [16, 19]]);
  assert.equal(participantHoldingAt(gap, 15), null);
});

test('a reused tag splits native runs into different countries and assigns each capitulation at its observed identity', () => {
  const item = war('reused', 10, 19);
  item.observations[0].viewpointSide = ['Reusable'];
  item.participants = [participant('Reusable', 'attacker', 10, 19)];
  item.capitulations = [{ tag: 'Reusable', hour: 12 }, { tag: 'Reusable', hour: 17 }];
  const lineage = new Lineage([{ hour: 15, from: 'Reusable', to: 'Renamed' }]);
  const data = prepareWars(reply([item]), lineage);
  assert.equal(data.countries.length, 2);
  assert.equal(data.wars[0].participants.length, 2);
  const old = data.wars[0].participants.find(view => view.key === lineage.countryAt('Reusable', 12))!;
  const replacement = data.wars[0].participants.find(view => view.key === lineage.countryAt('Reusable', 17))!;
  assert.deepEqual(old.runs.map(run => [run.firstHour, run.lastHour]), [[10, 14]]);
  assert.deepEqual(replacement.runs.map(run => [run.firstHour, run.lastHour]), [[15, 19]]);
  assert.deepEqual(old.capitulations, [12]);
  assert.deepEqual(replacement.capitulations, [17]);
  assert.equal(countryTagAt(old, 19), 'Renamed');
  assert.equal(countryTagAt(replacement, 19), 'Reusable');
  const shown = warsLanes(data, 19, { ...options, view: 'countries', named: new Set(['Reusable', 'Renamed']) });
  assert.deepEqual(shown.lanes.map(lane => lane.identity?.label), ['Name Renamed', 'Name Reusable']);
  assert.equal(new Set(shown.lanes.map(lane => lane.id)).size, 2);
});

test('simultaneous tag swaps move countries while retaining each run’s historical tag and side', () => {
  const item = war('swap', 10, 19);
  item.participants = [participant('Left tag', 'attacker', 10, 19), participant('Right tag', 'defender', 10, 19)];
  const lineage = new Lineage([{ hour: 15, from: 'Left tag', to: 'Right tag' }, { hour: 15, from: 'Right tag', to: 'Left tag' }]);
  const data = prepareWars(reply([item]), lineage);
  assert.equal(data.countries.length, 2);
  const leftCountry = data.wars[0].byCountry.get(lineage.countryAt('Left tag', 10))!;
  assert.deepEqual(leftCountry.runs.map(run => [run.tag, run.side, run.firstHour, run.lastHour]),
    [['Left tag', 'attacker', 10, 14], ['Right tag', 'defender', 15, 19]]);
  assert.equal(countryTagAt(leftCountry, 19), 'Right tag');
  const shown = warsLanes(data, 19, { ...options, expanded: new Set([item.id]), named: new Set(['Left tag', 'Right tag']) });
  const participants = shown.lanes.filter(lane => shown.info.get(lane.id)?.kind === 'participant');
  assert.deepEqual(participants.map(lane => lane.identity?.label), ['Name Left tag', 'Name Right tag']);
});

test('app war continuity can break at a rename without splitting the country lane or merging war categories', () => {
  const before = war('first-app-war', 10, 14);
  const after = war('second-app-war', 15, 19);
  before.participants = [participant('Before', 'attacker', 10, 14)];
  after.participants = [participant('After', 'attacker', 15, 19)];
  const data = prepareWars(reply([before, after]), new Lineage([{ hour: 15, from: 'Before', to: 'After' }]));
  assert.equal(data.wars.length, 2);
  assert.equal(data.countries.length, 1);
  const shown = warsLanes(data, 19, { ...options, view: 'countries', named: new Set(['After']) });
  assert.equal(shown.lanes.length, 1);
  assert.equal(shown.lanes[0].identity?.label, 'Name After');
  assert.deepEqual(shown.lanes[0].spans.map(span => [span.color, span.subrow]), [['first', 0], ['second', 0]]);
});

function large(count: number): WarsReply {
  const wars: WarTimeline[] = [];
  for (let index = 0; index < count / 10; index++) {
    const item = war(`war-${index}`, index, index + 100);
    item.participants = Array.from({ length: 10 }, (_, member) => participant(`country-${index * 10 + member}`, member % 2 ? 'defender' : 'attacker', index, index + 100));
    wars.push(item);
  }
  return reply(wars);
}

test('country views retain every war and span at 5,000 and 20,000 countries', () => {
  for (const count of [5_000, 20_000]) {
    const data = prepareWars(large(count));
    assert.equal(data.wars.length, count / 10);
    assert.equal(data.countries.length, count);
    const shown = warsLanes(data, count, { ...options, view: 'countries', named: new Set() });
    assert.equal(shown.lanes.length, count);
    assert.ok(shown.lanes.every(lane => lane.spans.length === 1));
  }
});

test('completed wars retain their final name, muting and orientation without filling internal gaps', () => {
  const item = war('finished', 10, 39);
  item.observations = [observation(10, 19), { ...observation(30, 39, false), name: { text: 'Final name', known: true, reason: null, key: null } }];
  item.participants = [participant('One side', 'defender', 30, 39), participant('Other side', 'attacker', 30, 39)];
  const prepared = prepareWars(reply([item]));
  assert.equal(labelObservationAt(item, 25), null);
  assert.equal(labelObservationAt(item, 50), item.observations[1]);
  const shown = warsLanes(prepared, 50, { ...options, expanded: new Set([item.id]) });
  assert.equal(shown.lanes[0].identity?.label, 'Final name');
  assert.equal(shown.lanes[0].muted, false);
  assert.deepEqual(shown.lanes.filter(lane => lane.labelStyle === 'side').map(lane => lane.identity?.label), ['Attackers', 'Defenders']);
  assert.equal(shown.lanes[2].identity?.label, 'Name Other side');
  item.observations[1].name = { text: 'Not recorded', known: false, reason: 'Missing operand', key: null };
  assert.equal(warsLanes(prepared, 50, options).lanes[0].muted, true);
});



test('Live introduces a war only at its start; the campaign end includes all wars', () => {
  const prepared = prepareWars(reply([war('first', 10, 19), war('future', 30, 39)]));
  for (const view of ['gantt', 'countries'] as const) {
    assert.equal(warsLanes(prepared, 9, { ...options, view }).lanes.length, 0);
    const now = warsLanes(prepared, 10, { ...options, view });
    assert.ok([...now.info.values()].every(lane => [...lane.spansById.values()].every(piece => piece.war.id === 'first')));
    const end = warsLanes(prepared, 40, { ...options, view });
    assert.equal(new Set([...end.info.values()].flatMap(lane => [...lane.spansById.values()].map(piece => piece.war.id))).size, 2);
  }
});




test('refetches keep existing expanded wars and drop only removed IDs', () => {
  const expanded = new Set(['first', 'second']);
  const refetched = prepareWars(reply([war('first'), war('second'), war('new')]));
  assert.equal(retainExpandedWars(expanded, refetched.byId), expanded);
  const removed = prepareWars(reply([war('second')]));
  assert.deepEqual([...retainExpandedWars(expanded, removed.byId)], ['second']);
  assert.deepEqual([...retainExpandedWars(expanded, prepareWars(reply([])).byId)], []);
});

test('wars the game names alike are told apart by their start; a unique name stays as recorded', () => {
  const first = war('1', 10, 40), second = { ...war('2', 30, 40), startedBy: false }, other = { ...war('3', 20, 40),
    observations: [{ ...observation(20, 40), name: { text: 'Other name', known: true, reason: null, key: null } }] };
  const names = warNamesAt([first, second, other], 35);
  assert.match(names.get(first)!, /^Recorded name \(by .+\)$/);
  assert.match(names.get(second)!, /^Recorded name \(from .+\)$/);
  assert.equal(names.get(other), 'Other name');
  assert.equal(warNamesAt([first, second], 20).get(first), 'Recorded name');
});

test('the lanes change only at structure hours, so a widget following the head renders only when it passes one', () => {
  const item = war('1', 10, 39);
  item.observations = [observation(10, 19), { ...observation(20, 39), name: { text: 'Second name', known: true, reason: null, key: null } }];
  item.participants = [participant('One side', 'attacker', 10, 39), participant('Other side', 'defender', 10, 39), participant('Third', 'defender', 25, 30)];
  const prepared = prepareWars(reply([item]));
  const hours = structureHours(prepared);
  assert.deepEqual(hours, [10, 20, 25, 31, 40]);
  for (let at = 0; at < 45; at++) {
    const from = hours[structureAt(hours, at) - 1] ?? at;
    const lanes = (hour: number) => JSON.stringify(warsLanes(prepared, hour, { ...options, expanded: new Set(['1']) }).lanes.map(lane => [lane.id, lane.identity?.label, lane.spans.length]));
    assert.equal(lanes(at), lanes(from), `hour ${at}`);
  }
  assert.equal(structureAt(hours, 9), 0);
  assert.equal(structureAt(hours, 24), structureAt(hours, 20));
  assert.notEqual(structureAt(hours, 25), structureAt(hours, 24));
});

test('a losses reply gives each war a run or the reason it has none, never both and never zero for unknown', () => {
  const data = { branchId: 'b', hour: 15, warnings: [], wars: [
    { id: '1', run: [15, 15, '10', '0', '-3', '2'], reason: null },
    { id: '2', run: null, reason: 'Not recorded: a relation' },
  ] };
  assert.equal(validateWarLosses(data), data);
  assert.deepEqual(lossesOf(data as never, '1')?.run, [15, 15, '10', '0', '-3', '2']);
  assert.equal(lossesOf(data as never, '2')?.run, null);
  assert.equal(lossesOf(data as never, '3'), undefined);
  for (const bad of [{ ...data, wars: [{ id: '1', run: null, reason: null }] }, { ...data, wars: [{ id: '1', run: [15, 15, '1x', '0', '0', '0'], reason: null }] },
    { ...data, wars: [data.wars[0], data.wars[0]] }, { ...data, hour: 'x' }]) assert.throws(() => validateWarLosses(bad), /malformed/);
  const groups = { branchId: 'b', hour: 15, available: true, warnings: [], contributions: [{ viewpointSide: ['A'], opposingSide: ['B'], reason: 'why', participants: [] }] };
  assert.equal(validateWarContributions(groups), groups);
  assert.throws(() => validateWarContributions({ ...groups, available: 'yes' }), /malformed/);
});

test('country view coloured by war gives more than eight wars distinct colours without wrapping', () => {
  const many = Array.from({ length: 12 }, (_, index) => war(`w${String(index).padStart(2, '0')}`, 10 + index, 40));
  const prepared = prepareWars(reply(many));
  const shown = warsLanes(prepared, 40, { ...options, view: 'countries', colour: 'war', palette: ['c0', 'c1', 'c2'], surface: '#16181d' });
  const spanColours = shown.lanes[0].spans.map(span => span.color);
  assert.deepEqual(spanColours.slice(0, 3), ['c0', 'c1', 'c2']);
  assert.equal(new Set(spanColours).size, 12);
  const earlier = warsLanes(prepared, 15, { ...options, view: 'countries', colour: 'war', palette: ['c0', 'c1', 'c2'], surface: '#16181d' });
  assert.deepEqual(earlier.lanes[0].spans.map(span => span.color), spanColours.slice(0, 6));
  const gantt = warsLanes(prepared, 40, { ...options, view: 'gantt', colour: 'war', expanded: new Set(['w09']), palette: ['c0', 'c1', 'c2'], surface: '#16181d' });
  assert.equal(gantt.lanes.find(lane => { const info = gantt.info.get(lane.id); return info?.kind === 'war' && info.war.id === 'w09'; })?.spans[0].color, 'neutral');
});
