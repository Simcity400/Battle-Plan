import assert from 'node:assert/strict';
import test from 'node:test';
import type { WarObservation, WarTimeline } from '../../generated/dashboard';
import { casualtyRows, warTooltip } from './warsTooltip.ts';
import { sideLabel } from './warsData.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import type { WidgetSpec } from '../../dashboard/model';

const country = (tag: string) => ({ name: `Name ${tag}`, known: true, flag: `flag:${tag}` });
const day = (hour: number) => `date ${hour}`;
const observation: WarObservation = { firstHour: 10, lastHour: 20, viewpointTag: 'opaque-player', viewpointSide: ['opaque-player'], opposingSide: ['other'],
  attackersAreViewpoint: true, representative: null, name: { text: 'Recorded title', known: true, reason: null, key: null } };
const war: WarTimeline = { id: 'opaque', firstHour: 10, lastHour: 20, startedBy: true, endedBy: true,
  observations: [observation], casualties: [], participants: [], capitulations: [] };
const input = { war, observation, losses: null, hour: 15, until: 40, branchLast: 40, unobserved: [], capitulationsUnobserved: [], country, day };

test('participant hovers show the war once and omit an absent optional capitulation row', () => {
  const content = warTooltip({ ...input, participant: { tag: 'other', run: {
    tag: 'other', side: 'defender', firstHour: 12, lastHour: 19, joinedBy: true, leftBy: true }, capitulations: [] } });
  assert.ok(!content.rows.some(row => row.label === 'War' || row.label === 'Capitulation'));
  assert.equal(content.subtitle, 'Recorded title · date 15');
  assert.equal(content.rows.find(row => row.label === 'Joined')?.value, 'by date 12');
  assert.equal(content.rows.find(row => row.label === 'Left')?.value, 'by date 20');
  assert.ok(!content.note?.includes('overview observations'));
});

test('wars follow the dashboard country; saved viewpoints migrate to it unless a country was picked', () => {
  assert.equal(definition.countries.max, 1);
  assert.equal('defaultCountries' in definition, false);
  assert.deepEqual(resolveOptions(OPTIONS, undefined), { view: 'gantt', colour: 'war', grow: false });
  const player = { countries: { kind: 'tags', tags: [] }, options: { viewpoint: 'player', view: 'countries' } } as unknown as WidgetSpec;
  assert.deepEqual(definition.migrate(2, player), { countries: { kind: 'selected' }, options: { view: 'countries' } });
  const unpicked = { countries: { kind: 'tags', tags: ['opaque picked'] }, options: { viewpoint: 'player' } } as unknown as WidgetSpec;
  assert.deepEqual(definition.migrate(2, unpicked)?.countries, { kind: 'selected' });
  const picked = { countries: { kind: 'tags', tags: ['opaque picked'] }, options: { viewpoint: 'country' } } as unknown as WidgetSpec;
  assert.equal(definition.migrate(2, picked)?.countries, picked.countries);
});

test('finished-war hover retains the last name and orientation without extending rosters or losses', () => {
  const content = warTooltip({ ...input, hour: 30, observation: null, losses: null });
  assert.equal(content.title, 'Recorded title');
  assert.equal(content.rows.find(row => row.label === 'Participants')?.value, 'Ended');
  assert.equal(content.rows.find(row => row.label === 'Attacker casualties')?.value, 'Ended');
  assert.equal(content.rows.find(row => row.label === 'Defender casualties')?.value, 'Ended');
  assert.equal(content.rows.filter(row => row.indent).length, 0);
});

test('recorded hours before a war say not started without inventing a roster or casualties', () => {
  const content = warTooltip({ ...input, hour: 5, observation: null });
  assert.equal(content.title, 'Not started');
  assert.equal(content.description, undefined);
  for (const label of ['Participants', 'Viewpoint’s side casualties', 'Opposing side casualties']) {
    assert.equal(content.rows.find(row => row.label === label)?.value, 'Not started');
  }
  assert.equal(content.rows.filter(row => row.indent).length, 0);
});

test('a participant outside its runs is absent from this war at recorded hours', () => {
  const run = { tag: 'other', side: 'defender' as const, firstHour: 12, lastHour: 19, joinedBy: true, leftBy: true };
  for (const hour of [5, 10, 20, 30]) {
    const content = warTooltip({ ...input, hour, observation: hour < 10 || hour > 20 ? null : observation,
      participant: { tag: 'other', run, capitulations: [18] } });
    for (const label of ['Side', 'Joined', 'Left', 'Capitulation']) {
      assert.equal(content.rows.find(row => row.label === label)?.value, 'Not in this war at this hour');
    }
    assert.equal(content.rows.filter(row => row.label === 'Capitulated').length, 0);
  }
});

test('overview and branch gaps stay unknown even before the start or after the end', () => {
  for (const hour of [5, 15, 30]) for (const gaps of ['unobserved', 'branchGaps'] as const) {
    const params = { ...input, hour, observation: null, [gaps]: [{ firstHour: hour, lastHour: hour }] };
    const content = warTooltip(params);
    assert.equal(content.title, 'Not recorded');
    for (const label of ['Participants', 'Viewpoint’s side casualties', 'Opposing side casualties']) {
      assert.equal(content.rows.find(row => row.label === label)?.value, 'Not recorded');
    }
    const member = warTooltip({ ...params, participant: { tag: 'other', run: null } });
    for (const label of ['Side', 'Joined', 'Left', 'Capitulation']) {
      assert.equal(member.rows.find(row => row.label === label)?.value, 'Not recorded');
    }
  }
});

test('unknown viewpoint names stay neutral in casualty and roster divider labels', () => {
  const content = warTooltip({ ...input, observation: { ...observation, attackersAreViewpoint: null },
    country: () => ({ name: 'Loading…' }) });
  assert.ok(content.rows.some(row => row.label === 'Viewpoint’s side casualties'));
  assert.ok(content.rows.some(row => row.divider && row.label === 'Viewpoint’s side'));
  assert.ok(content.rows.every(row => !row.label.includes('Loading…’s')));
});

test('participant and side tooltips distinguish actual read status from authored UI-word names', () => {
  const participant = { tag: 'other', run: { tag: 'other', side: 'viewpoint' as const, firstHour: 10, lastHour: 20, joinedBy: true, leftBy: false } };
  for (const name of ['Loading…', 'Not recorded', '', 'Named country']) {
    const content = warTooltip({ ...input, observation: { ...observation, attackersAreViewpoint: null }, participant,
      country: () => ({ name, known: true, nameFailed: false }) });
    assert.equal(content.title, name);
    assert.equal(content.rows.find(row => row.label === 'Side')?.value, `${name}’s side`);
    assert.equal(content.description, undefined);
  }
  for (const known of [undefined, false]) {
    const content = warTooltip({ ...input, participant, country: () => ({ name: known === false ? 'Not recorded' : 'Loading…', known }) });
    assert.equal(content.description, undefined);
  }
});

test('active war and participant wording describes fighting and the recording end', () => {
  const member = { tag: 'other', side: 'defender' as const, firstHour: 10, lastHour: 20, joinedBy: true, leftBy: true };
  assert.equal(warTooltip({ ...input, until: 15 }).rows.find(row => row.label === 'Ended')?.value, 'Ongoing');
  assert.equal(warTooltip({ ...input, until: 15, participant: { tag: 'other', run: member } }).rows.find(row => row.label === 'Left')?.value, 'Still fighting');
  for (const participant of [undefined, { tag: 'other', run: member }]) {
    const content = warTooltip({ ...input, until: 20, branchLast: 20, participant });
    assert.equal(content.rows.find(row => row.label === (participant ? 'Left' : 'Ended'))?.value, 'Ongoing at the recording’s end');
  }
});

test('casualty side wording is singular and dates identify the one covering read', () => {
  const content = warTooltip({ ...input, losses: [10, 20, '1234', '0', '5678', '1'] });
  assert.deepEqual(content.rows.filter(row => row.label.endsWith('casualties')).map(row => row.label),
    ['Attacker casualties', 'Defender casualties', 'Defender unattributed casualties']);
  assert.match(content.note!, /Casualties as recorded date 10 – date 20/);
});

test('casualties retain all signed integer digits and unknown attribution separately', () => {
  assert.deepEqual(casualtyRows([10, 20, '900719925474099312345', '-12', '0', '0'], ['Attackers', 'Defenders']).map(row => row.value),
    ['900,719,925,474,099,312,345', '−12', '0']);
  assert.ok(casualtyRows(null, ['Attackers', 'Defenders']).every(row => row.value === 'Not recorded'));
});

test('unknown orientation names the viewpoint and never guesses attackers', () => {
  assert.deepEqual(['viewpoint', 'opponent'].map(side => sideLabel(side as 'viewpoint' | 'opponent', { ...observation, attackersAreViewpoint: null }, country)), ['Name opaque-player’s side', 'Opposing side']);
  assert.deepEqual(['viewpoint', 'opponent'].map(side => sideLabel(side as 'viewpoint' | 'opponent', { ...observation, attackersAreViewpoint: false }, country)), ['Defenders', 'Attackers']);
});

test('boundaries say by and missing hours never imply an exact end', () => {
  const rows = warTooltip({ ...input, unobserved: [{ firstHour: 21, lastHour: 30 }] }).rows;
  assert.equal(rows.find(row => row.label === 'Started')?.value, 'by date 10');
  assert.equal(rows.find(row => row.label === 'Ended')?.value, 'by date 31');
  assert.equal(warTooltip({ ...input, until: 25, unobserved: [{ firstHour: 21, lastHour: 30 }] }).rows[1].value, 'Not recorded; last seen date 20');
  assert.equal(warTooltip({ ...input, until: 20 }).rows[1].value, 'Ongoing');
  assert.equal(warTooltip({ ...input, war: { ...war, endedBy: false } }).rows[1].value, 'date 21');
});

test('participant tooltip has dated side, by bounds, capitulation and missing-capitulation note', () => {
  const run = { tag: 'other', side: 'defender' as const, firstHour: 12, lastHour: 19, joinedBy: true, leftBy: true };
  const content = warTooltip({ ...input, war: { ...war, capitulations: [{ tag: 'other', hour: 18 }] }, participant: { tag: 'other', run },
    capitulationsUnobserved: [{ firstHour: 13, lastHour: 14 }] });
  assert.equal(content.title, 'Name other');
  assert.equal(content.countryFlag?.flag, 'flag:other');
  assert.equal(content.rows.find(row => row.label === 'Joined')?.value, 'by date 12');
  assert.equal(content.rows.find(row => row.label === 'Left')?.value, 'by date 20');
  assert.equal(content.rows.find(row => row.label === 'Side')?.value, 'Defender');
  assert.equal(content.rows.find(row => row.label === 'Capitulated')?.value, 'date 18');
  assert.match(content.note!, /Some capitulations may be missing/);
});

test('native roster and participant flags retain pending and refused reads without marking war or side headings', () => {
  const country = (tag: string) => ({ name: `Name ${tag}`, flagReason: `Flag unavailable for ${tag}`, flagPending: tag === 'other' });
  const overview = warTooltip({ ...input, country });
  assert.equal(overview.countryFlag, undefined);
  assert.ok(overview.rows.filter(row => row.divider).every(row => row.countryFlag === undefined));
  assert.deepEqual(overview.rows.find(row => row.label === 'Name other')?.countryFlag, country('other'));
  assert.deepEqual(overview.rows.find(row => row.label === 'Name opaque-player')?.countryFlag, country('opaque-player'));
  assert.deepEqual(warTooltip({ ...input, country, participant: { tag: 'other', run: null } }).countryFlag, country('other'));
});

test('missing name and missing observation are explicit without backfilled participants or losses', () => {
  const unknown = warTooltip({ ...input, observation: { ...observation, name: { text: 'Not recorded', known: false, reason: 'Missing naming operand', key: null } } });
  assert.equal(unknown.title, 'Not recorded');
  assert.equal(unknown.description, undefined);
  const gap = warTooltip({ ...input, observation: null });
  assert.equal(gap.rows.find(row => row.label === 'Participants')?.value, 'Not recorded');
  assert.equal(gap.rows.filter(row => row.indent).length, 0);
});

test('hovering an unobserved participant hour does not carry a historical capitulation', () => {
  const gap = warTooltip({ ...input, observation: null, war: { ...war, capitulations: [{ tag: 'other', hour: 18 }] },
    participant: { tag: 'other', run: null } });
  assert.equal(gap.rows.filter(row => row.label === 'Capitulated').length, 0);
  assert.equal(gap.rows.find(row => row.label === 'Capitulation')?.value, 'Not recorded');
});

test('participant holding keeps renamed country capitulations and filters other stretches', () => {
  const run = { tag: 'previous', side: 'defender' as const, firstHour: 12, lastHour: 19, joinedBy: true, leftBy: true };
  const content = warTooltip({ ...input, participant: { tag: 'renamed', run, capitulations: [9, 18, 21] } });
  assert.equal(content.title, 'Name renamed');
  assert.equal(content.countryFlag?.flag, 'flag:renamed');
  assert.deepEqual(content.rows.filter(row => row.label === 'Capitulated').map(row => row.value), ['date 18']);
  assert.equal(content.rows.find(row => row.label === 'Joined')?.value, 'by date 12');
  assert.equal(content.rows.find(row => row.label === 'Left')?.value, 'by date 20');
  const gap = warTooltip({ ...input, participant: { tag: 'renamed', run: null, capitulations: [18] } });
  assert.equal(gap.rows.filter(row => row.label === 'Capitulated').length, 0);
});


test('exact casualty reads show loading while known war geometry stays available', () => {
  const war = {id:'1',firstHour:0,lastHour:10,startedBy:true,endedBy:true,observations:[],casualties:[],participants:[],capitulations:[]};
  const observation = {firstHour:0,lastHour:10,viewpointTag:'A',viewpointSide:['A'],opposingSide:['B'],attackersAreViewpoint:true,
    representative:['A','B'] as [string,string],name:{text:'Recorded war',known:true,reason:null,key:null}};
  const base={war,observation,losses:null,lossesPending:true,hour:5,until:10,branchLast:10,unobserved:[],capitulationsUnobserved:[],
    country:(tag:string)=>({name:tag,known:true}),day:(hour:number)=>String(hour)};
  const pending=warTooltip(base);
  assert.equal(pending.title,'Recorded war');
  assert.ok(pending.rows.filter(row=>row.label?.endsWith('casualties')).every(row=>row.value==='Loading…'));
  const gap=warTooltip({...base,unobserved:[{firstHour:5,lastHour:5}]});
  assert.ok(gap.rows.filter(row=>row.label?.endsWith('casualties')).every(row=>row.value==='Not recorded'));
});

test('a war reaching the prepared prefix has no end yet, and hours after the prefix are still being prepared', () => {
  const open = warTooltip({ ...input, hour: 15, preparedThrough: 20 });
  assert.equal(open.rows.find(row => row.label === 'Ended')?.value, 'Still being prepared');
  const after = warTooltip({ ...input, hour: 25, preparedThrough: 20 });
  assert.equal(after.rows.find(row => row.label === 'Participants')?.value, 'Still being prepared');
  // Without a prefix the last observation is the end bound, as before.
  assert.equal(warTooltip({ ...input, hour: 25 }).rows.find(row => row.label === 'Participants')?.value, 'Ended');
});
