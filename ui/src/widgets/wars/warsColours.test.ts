import assert from 'node:assert/strict';
import test from 'node:test';
import { lookPlan } from '../../dashboard/countryLooks.ts';
import { sampledLooks } from '../../dashboard/looks.ts';
import { Lineage } from '../../dashboard/identity.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { chosenColour, factionIdentity, parseCampaignSettings, setFactionColour } from '../../dashboard/campaignSettings.ts';
import { factionColours } from '../../charts/factionColours.ts';
import { FAILED_FACT, nameKey } from '../../dashboard/countryFacts.ts';
import { parseColor, rgbToOklch } from '../../shell/appearance/color.ts';
import type { CountryGroupReply, CountryStyle, FactionGroup } from '../../generated/dashboard';
import type { WarTimeline, WarsReply } from '../../generated/dashboard';
import { prepareWars, warsLanes } from './warsData.ts';
import type { WarsLaneOptions } from './warsData';
import { warsColourPlan, warsColourSelection, warsCountryColours, warsFactionMembership, warsFixedColourKey, warsLookStart, warsSideColours } from './warsColours.ts';

const faction = (id: string, members: FactionGroup['members']): FactionGroup => ({
  id, firstHour: 10, lastHour: 39, members, incomplete: [], leaders: [], name: null,
  recordedName: '', nameReason: null, names: [],
});
const groups = (factions: FactionGroup[]): CountryGroupReply => ({
  branchId: 'branch', factions, humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], countryIdentities: [],
  unobserved: { humans: [], factions: [], wars: [] }, warnings: [],
});
const warReply = (members: string[]): WarsReply => ({
  branchId: 'branch', available: true, viewpoints: [], warnings: [], countryNames: [], unobserved: [], capitulationsUnobserved: [],
  wars: [{
    id: 'war', firstHour: 10, lastHour: 39, startedBy: true, endedBy: false, capitulations: [],
    observations: [{ firstHour: 10, lastHour: 39, viewpointTag: 'First', viewpointSide: members, opposingSide: ['Opponent'],
      attackersAreViewpoint: true, representative: ['First', 'Opponent'], name: { text: 'Synthetic war', known: true, reason: null, key: null } }],
    participants: [...members.map(tag => ({ tag, side: 'attacker' as const, firstHour: 10, lastHour: 39, joinedBy: true, leftBy: false })),
      { tag: 'Opponent', side: 'defender', firstHour: 10, lastHour: 39, joinedBy: true, leftBy: false }],
  }],
});
const laneOptions: WarsLaneOptions = {
  view: 'countries', colour: 'side', expanded: new Set(), palette: ['attack', 'defence'], neutral: 'neutral',
  named: new Set(['First', 'Second', 'Ally', 'Opponent']), country: tag => ({ name: tag }),
};
const roster = (tags: string[]): FactionGroup['members'] => tags.map(tag => ({ tag, firstHour: 10, lastHour: 39 }));

test('native holder bounds reset country looks and incomplete identities cannot prove a faction majority', () => {
  const membership = groups([faction('91', roster(['First', 'Second']))]);
  membership.countryIdentities = ['First', 'Second', 'Ally', 'Opponent'].map((tag, index) => ({ tag,
    identities: [{ firstHour: 10, lastHour: 19, slot: index, countryId: index + 10 },
      { firstHour: 30, lastHour: 39, slot: index, countryId: index + 20 }] }));
  membership.countryIdentities.push({ tag: 'After war', identities: [{ firstHour: 40, lastHour: 49, slot: 0, countryId: 20 }] });
  const prepared = prepareWars(warReply(['First', 'Second', 'Ally']), new Lineage([]), countryIdentityIndex(membership.countryIdentities));
  const branch = { firstHour: 10, lastHour: 39, gaps: [] };
  assert.equal(warsLookStart(prepared, branch, 'First', 12), 10);
  assert.equal(warsLookStart(prepared, branch, 'First', 25), null);
  assert.equal(warsLookStart(prepared, branch, 'First', 32), 30);
  assert.equal(warsLookStart(prepared, { ...branch, lastHour: 49 }, 'After war', 45), 40);
  assert.ok([...warsColourSelection(prepared, 45).values()].includes('After war'));
  assert.equal(warsSideColours(prepared, membership, 12, new Map([['91', '#123456']])).get('war')?.get('viewpoint'), '#123456');
  assert.equal(warsSideColours(prepared, membership, 25, new Map([['91', '#123456']])).size, 0);
});

test('a monofaction side uses its recorded grey, global Axis swatch and campaign override', () => {
  const data = prepareWars(warReply(['First', 'Second']));
  const membership = groups([faction('91', roster(['First', 'Second']))]);
  const identity = factionIdentity('Axis', 'First');
  const global = setFactionColour(parseCampaignSettings(undefined), 'all', 'aa', '91', identity, '#123456');
  const local = setFactionColour(global, 'campaign', 'aa', '91', identity, '#654321');
  for (const [settings, expected] of [[parseCampaignSettings(undefined), '#808080'], [global, '#123456'], [local, '#654321']] as const) {
    const colours = factionColours([{ id: '91', leaderColor: parseColor('#808080') }], { 91: chosenColour(settings, 'aa', '91', identity) }, '#16181d');
    const sides = warsSideColours(data, membership, 20, colours);
    const shown = warsLanes(data, 20, { ...laneOptions, sideColours: sides });
    for (const tag of ['First', 'Second', 'Opponent']) {
      const lane = shown.lanes.find(lane => {
        const info = shown.info.get(lane.id);
        return info?.kind === 'country' && info.tag === tag;
      });
      assert.equal(lane?.spans[0].color, tag === 'Opponent' ? 'defence' : expected);
    }
    const byWar = warsLanes(data, 20, { ...laneOptions, colour: 'war', sideColours: sides });
    assert.ok(byWar.lanes.every(lane => lane.spans[0].color === 'attack'));
    const gantt = warsLanes(data, 20, { ...laneOptions, view: 'gantt', sideColours: sides });
    assert.equal(gantt.lanes[0].spans[0].color, 'neutral');
  }
});

test('a strict country majority includes factionless allies and leaves exactly half on the palette', () => {
  const colours = new Map([['91', '#123456'], ['508', '#abcdef']]);
  const membership = groups([faction('91', roster(['First', 'Second']))]);
  for (const [members, expected] of [[['First', 'Second', 'Ally'], '#123456'], [['First', 'Ally'], 'attack']] as const) {
    const data = prepareWars(warReply([...members]));
    const shown = warsLanes(data, 20, { ...laneOptions, sideColours: warsSideColours(data, membership, 20, colours) });
    assert.equal(shown.lanes[0].spans[0].color, expected);
  }
  membership.factions.push(faction('508', roster(['Ally'])));
  const tied = prepareWars(warReply(['First', 'Ally']));
  assert.equal(warsLanes(tied, 20, { ...laneOptions, sideColours: warsSideColours(tied, membership, 20, colours) }).lanes[0].spans[0].color, 'attack');
});

test('side colours follow dated membership and native arrays, with no final-read fill for unknown hours', () => {
  const data = prepareWars(warReply(['First', 'Second', 'Ally']));
  const membership = groups([
    faction('91', ['First', 'Second'].map(tag => ({ tag, firstHour: 10, lastHour: 19 }))),
    faction('508', ['First', 'Second'].map(tag => ({ tag, firstHour: 30, lastHour: 39 }))),
  ]);
  const colours = new Map([['91', '#123456'], ['508', '#abcdef']]);
  const first = () => warsLanes(data, 12, { ...laneOptions, sideColours: warsSideColours(data, membership, 12, colours) }).lanes[0].spans[0].color;
  assert.equal(first(), '#123456');
  assert.equal(warsSideColours(data, membership, 35, colours).get('war')?.get('viewpoint'), '#abcdef');
  assert.equal(first(), '#123456');
  assert.equal(warsSideColours(data, membership, 25, colours).size, 0);
  membership.unobserved.factions = [{ firstHour: 20, lastHour: 29 }];
  assert.equal(warsSideColours(data, membership, 25, colours).size, 0);
  membership.factions[1].incomplete = [{ firstHour: 32, lastHour: 34 }];
  assert.equal(warsSideColours(data, membership, 33, colours).size, 0);
  assert.equal(warsSideColours(data, undefined, 12, colours).size, 0);
  assert.equal(warsSideColours(data, membership, 45, colours).size, 0);
  const item: WarTimeline = data.reply.wars[0];
  item.observations[0].viewpointSide = ['Opponent'];
  item.observations[0].opposingSide = ['First', 'Second', 'Ally'];
  item.observations[0].attackersAreViewpoint = false;
  const swapped = prepareWars(data.reply);
  assert.equal(warsSideColours(swapped, membership, 12, colours).get('war')?.get('opponent'), '#123456');
  assert.equal(warsLanes(swapped, 12, { ...laneOptions, sideColours: warsSideColours(swapped, membership, 12, colours) }).lanes[0].spans[0].color, '#123456');
});

test('repeated native side references count each participating country once', () => {
  const data = prepareWars(warReply(['First', 'First', 'Ally']));
  const membership = groups([faction('91', roster(['First']))]);
  assert.equal(warsSideColours(data, membership, 20, new Map([['91', '#123456']])).size, 0);
});

test('participant colours preserve raw grey and opt into fixed colours in stable opaque-tag order', () => {
  const tags = ['𐀀', '\uE000', 'AAA', 'aaa'];
  const colour = (): CountryStyle['color'] => [128, 128, 128];
  const raw = warsCountryColours(tags, colour, '#16181d', false);
  assert.ok([...raw.values()].every(value => value === '#808080'));
  const fixed = warsCountryColours(tags, colour, '#16181d', true);
  assert.deepEqual(warsCountryColours([...tags].reverse(), colour, '#16181d', true), fixed);
  assert.equal(new Set(fixed.values()).size, tags.length);
  assert.ok([...fixed.values()].every(value => rgbToOklch(parseColor(value)!).c > 0.025));
  assert.deepEqual(warsCountryColours(tags, colour, '#16181d', false), raw);
});

test('fixed participant colours use the same full identity selection in different viewports and widgets', () => {
  const data = prepareWars(warReply(['A', 'B']));
  const selection = warsColourSelection(data, 20);
  const grey = (key: string): CountryStyle['color'] => selection.get(key) === 'Opponent' ? null : [128, 128, 128];
  const colours = warsCountryColours([...selection.keys()], grey, '#16181d', true);
  const show = (named: string[], corrected = colours) => warsLanes(data, 20, {
    ...laneOptions, view: 'gantt', expanded: new Set(['war']), named: new Set(named),
    country: tag => ({ name: tag, color: corrected.get(data.lineage.countryAt(tag, 20)) }),
  }).lanes.find(lane => lane.identity?.label === 'B')!.spans[0].color;
  assert.equal(show(['B']), show(['A', 'B']));
  const independent = prepareWars(warReply(['A', 'B']));
  const otherSelection = warsColourSelection(independent, 20);
  const otherColours = warsCountryColours([...otherSelection.keys()].reverse(), grey, '#16181d', true);
  assert.equal(show(['A', 'B'], otherColours), show(['B']));
});

test('full colour selection dates renamed and reused tags without assigning a current holder to an earlier identity', () => {
  const before = warReply(['Before']).wars[0];
  before.firstHour = 10; before.lastHour = 14;
  before.observations[0].firstHour = 10; before.observations[0].lastHour = 14;
  before.participants.forEach(run => { run.firstHour = 10; run.lastHour = 14; });
  const after = warReply(['Before']).wars[0];
  after.id = 'new holder'; after.firstHour = 20;
  after.observations[0].firstHour = 20;
  after.participants.forEach(run => { run.firstHour = 20; });
  const lineage = new Lineage([{ hour: 15, from: 'Before', to: 'After' }]);
  const data = prepareWars({ ...warReply([]), wars: [before, after] }, lineage);
  const earlier = lineage.countryAt('Before', 10), later = lineage.countryAt('Before', 20);
  assert.equal(warsColourSelection(data, 12).get(earlier), 'Before');
  assert.equal(warsColourSelection(data, 12).get(later), null);
  assert.equal(warsColourSelection(data, 20).get(earlier), 'After');
  assert.equal(warsColourSelection(data, 20).get(later), 'Before');
  assert.deepEqual([...warsColourSelection(data, 12).keys()], [...warsColourSelection(data, 20).keys()]);
  const branch = { firstHour: 0, lastHour: 39, gaps: [{ firstHour: 16, lastHour: 19 }] };
  assert.deepEqual(warsColourPlan(data, branch, 12, false).wanted, []);
  assert.equal(warsColourPlan(data, branch, 18, true).wanted.length, 0);
  const planned = warsColourPlan(data, branch, 20, true).samples.filter(sample => sample.tag === 'Before' || sample.tag === 'After');
  assert.deepEqual(planned.map(sample => [sample.tag, sample.hour]), [['After', 20], ['Before', 20]]);
  assert.equal(warsColourPlan(data, { ...branch, gaps: [] }, 20, true).samples.find(sample => sample.tag === 'Before')?.hour, 15);
});

test('new recorded colours replace previous colours and gaps keep the neutral fallback', () => {
  const tags = ['Member', 'Not captured'];
  for (const fixed of [false, true]) {
    const before = warsCountryColours(tags, tag => tag === 'Member' ? [19, 41, 73] : null, '#16181d', fixed);
    const after = warsCountryColours(tags, tag => tag === 'Member' ? [183, 73, 19] : null, '#16181d', fixed);
    assert.notEqual(before.get('Member'), after.get('Member'));
    assert.equal(before.has('Not captured'), false);
    assert.equal(after.has('Not captured'), false);
    assert.equal(warsCountryColours(tags, () => null, '#16181d', fixed).size, 0);
  }
});

test('fixed correction waits for all 5,000-country style answers and ignores unchanged hours and names', () => {
  const tags = Array.from({ length: 5_000 }, (_, index) => `Synthetic member ${index}`);
  const data = prepareWars(warReply(tags));
  const branch = { firstHour: 0, lastHour: 39, gaps: [] };
  const plan = warsColourPlan(data, branch, 20, true);
  const styles = new Map<string, CountryStyle | null | Error>();
  for (const count of [0, 64, 4_999]) {
    for (const item of plan.wanted.slice(styles.size, count)) styles.set(nameKey(item.hour, item.tag), { tag: item.tag, name: null, color: [19, 41, 73] });
    assert.equal(warsFixedColourKey(plan, styles), null);
  }
  for (const item of plan.wanted.slice(styles.size)) styles.set(nameKey(item.hour, item.tag), null);
  const key = warsFixedColourKey(plan, styles);
  assert.notEqual(key, null);
  assert.equal(warsFixedColourKey(warsColourPlan(data, branch, 21, true), styles), key);
  const first = plan.wanted[0], firstKey = nameKey(first.hour, first.tag);
  styles.set(firstKey, { tag: first.tag, name: 'Renamed display label', color: [19, 41, 73] });
  assert.equal(warsFixedColourKey(plan, styles), key);
  styles.set(firstKey, { tag: first.tag, name: null, color: [183, 73, 19] });
  assert.notEqual(warsFixedColourKey(plan, styles), key);
  for (const item of plan.wanted) styles.set(nameKey(item.hour, item.tag), FAILED_FACT);
  assert.equal(warsFixedColourKey(plan, styles), null);
});

test('faction membership follows selected-hour changes, renames and gaps without final-hour backfill', () => {
  const lineage = new Lineage([{ hour: 15, from: 'Before', to: 'After' }]);
  const data = groups([
    faction('7', [{ tag: 'Before', firstHour: 10, lastHour: 14 }, { tag: 'After', firstHour: 15, lastHour: 19 }]),
    faction('93', [{ tag: 'After', firstHour: 30, lastHour: 39 }]),
  ]);
  const country = lineage.countryAt('Before', 10);
  assert.equal(warsFactionMembership(data, 12, lineage)?.get(country), '7');
  assert.equal(warsFactionMembership(data, 16, lineage)?.get(country), '7');
  assert.equal(warsFactionMembership(data, 25, lineage)?.has(country), false);
  assert.equal(warsFactionMembership(data, 35, lineage)?.get(country), '93');
  data.unobserved.factions = [{ firstHour: 20, lastHour: 29 }];
  assert.equal(warsFactionMembership(data, 25, lineage), null);
  data.factions[1].incomplete = [{ firstHour: 32, lastHour: 34 }];
  assert.equal(warsFactionMembership(data, 33, lineage), null);
  assert.equal(warsFactionMembership(undefined, 12, lineage), null);
});

test('membership and raw country colours retain all 5,000 synthetic countries without repeated roster scans', () => {
  const tags = Array.from({ length: 5_000 }, (_, index) => `Synthetic member ${index}`);
  let reads = 0;
  const data = groups([faction('5009', tags.map(tag => ({
    tag, get firstHour() { reads++; return 10; }, lastHour: 39,
  })))]);
  const membership = warsFactionMembership(data, 20, new Lineage([]));
  assert.equal(membership?.size, tags.length);
  assert.equal(reads, tags.length);
  assert.equal(warsCountryColours(tags, () => [19, 41, 73], '#16181d', false).size, tags.length);
  reads = 0;
  const prepared = prepareWars(warReply(tags));
  assert.equal(warsColourPlan(prepared, { firstHour: 0, lastHour: 39, gaps: [] }, 20, true).wanted.length, tags.length + 1);
  const sideColours = warsSideColours(prepared, data, 20, new Map([['5009', '#132949']]));
  const shown = warsLanes(prepared, 20, { ...laneOptions, named: new Set(), sideColours });
  assert.equal(shown.lanes.length, tags.length + 1);
  assert.equal(shown.lanes.filter(lane => lane.spans[0].color === '#132949').length, tags.length);
  assert.equal(reads, tags.length);
});


test('lane names, flags and style plans share a reused native tag holder boundary', () => {
  const data = prepareWars(warReply(['Alias']), new Lineage([{ hour: 15, from: 'Alias', to: 'Renamed' }]));
  const branch = { firstHour: 10, lastHour: 39, gaps: [] };
  assert.equal(warsLookStart(data, branch, 'Alias', 14), 10);
  assert.equal(warsLookStart(data, branch, 'Alias', 15), 15);
  const plan = lookPlan(branch, ['Alias'], 20, (tag, hour) => warsLookStart(data, branch, tag, hour), undefined, true);
  assert.deepEqual(plan.samples, [{ tag: 'Alias', hour: 20, stretch: 15 }]);
  const pending = sampledLooks({ looks: new Map([['Alias', { name: 'Previous holder', flag: 'old flag', color: null }]]), segments: new Map([['Alias', 10]]) },
    plan.samples, () => ({ style: undefined, flag: undefined }));
  assert.equal(pending.looks.has('Alias'), false, 'new pending reads never borrow the older actor');
  assert.equal(warsLookStart(data, { ...branch, firstHour: 18 }, 'Alias', 20), 18);
});
