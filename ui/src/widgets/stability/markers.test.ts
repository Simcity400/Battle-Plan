import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply, FocusReply, IdeaHistoryReply } from '../../generated/dashboard';
import { stabilityMarkers } from './markers.ts';
import type { MarkerKind } from './markers.ts';

const ALL = new Set<MarkerKind>(['wars', 'factions', 'government', 'focus']);
const names: Record<string, string> = { GER: 'Germany', POL: 'Poland', FRA: 'France', ENG: 'United Kingdom' };
const base = { tag: 'GER', extent: [0, 1000] as const, branch: [0, 1000] as const, countryName: (tag: string) => names[tag] ?? tag };

const groups: CountryGroupReply = {
  branchId: '1', humans: [], warnings: [], countryIdentities: [],
  factions: [
    { id: '1', name: 'Axis', recordedName: 'Axis', firstHour: 100, lastHour: 1000, nameReason: null, names: [{ firstHour: 100, lastHour: 1000, name: 'Axis', reason: null }], members: [{ tag: 'GER', firstHour: 100, lastHour: 1000 }, { tag: 'ITA', firstHour: 150, lastHour: 1000 }], leaders: [], incomplete: [] },
    { id: '2', name: 'Pact', recordedName: 'Pact', firstHour: 0, lastHour: 1000, nameReason: null, names: [{ firstHour: 0, lastHour: 1000, name: 'Pact', reason: null }], members: [{ tag: 'GER', firstHour: 300, lastHour: 399 }], leaders: [], incomplete: [] },
  ],
  wars: [{ first: 'FRA', second: 'GER', firstHour: 200, lastHour: 500 }, { first: 'GER', second: 'POL', firstHour: 180, lastHour: 1000 }],
  warStarts: [{ hour: 180, attacking: 'GER', defending: 'POL' }, { hour: 200, attacking: 'FRA', defending: 'GER' }],
  capitulations: [{ hour: 450, tag: 'FRA' }, { hour: 460, tag: 'ENG' }], identityChanges: [],
  unobserved: { humans: [], factions: [{ firstHour: 400, lastHour: 400 }], wars: [] },
};

test('wars, capitulations and faction changes are dated as recorded; a gap is not a leave', () => {
  const badges = stabilityMarkers({ ...base, groups }, ALL);
  assert.deepEqual(badges.map(badge => [badge.hour, badge.glyph, badge.label]), [
    [100, '⚑', 'Founded Axis'],
    [180, '⚔', 'War with Poland began'],
    [200, '⚔', 'War with France began'],
    [300, '⚑', 'Joined Pact'],
    [450, '⚔', 'France capitulated'],
  ]);
  // Only the kinds switched on, only inside the hours shown.
  assert.deepEqual(stabilityMarkers({ ...base, groups, extent: [150, 250] }, new Set(['wars'])).map(badge => badge.label), ['War with Poland began', 'War with France began']);
});

test('laws and advisors taken on while recorded are government changes; focuses completed are marked', () => {
  const ideas = {
    branchId: '1', tag: 'GER', warnings: [], unobserved: [{ firstHour: 599, lastHour: 599 }],
    spans: [
      { token: 'civilian_economy', firstHour: 0, lastHour: 499, slot: null, character: null },
      { token: 'war_economy', firstHour: 500, lastHour: 1000, slot: null, character: null },
      { token: 'total_mobilisation', firstHour: 600, lastHour: 1000, slot: null, character: null },
      { token: 'speer', firstHour: 700, lastHour: 1000, slot: 'political_advisor', character: { id: 7, token: 'GER_albert_speer', name: 'GER_albert_speer' } },
    ],
    ideas: ['civilian_economy', 'war_economy', 'total_mobilisation'].map(token => ({ token, category: 'government', slot: 'economy', law: true, designer: false, effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null })),
    artwork: [{ token: 'war_economy', name: 'War Economy', description: null, baseDurationDays: null, icon: null }],
    holders: [{ token: 'speer', character: 7, characterToken: 'GER_albert_speer', name: 'GER_albert_speer', artwork: { token: 'speer', name: 'Albert Speer', description: null, baseDurationDays: null, icon: null } }],
    categories: [{ token: 'government', name: null, kind: null, hidden: false, politicsTab: true, slots: [
      { token: 'economy', name: null, character: false, seats: 1, laws: ['civilian_economy', 'war_economy', 'total_mobilisation'] },
      { token: 'political_advisor', name: null, character: true, seats: 3, laws: [] }] }],
  } as unknown as IdeaHistoryReply;
  const focus = { warnings: [], artwork: [], staticText: [{ token: 'GER_rhineland', name: 'Rhineland', description: null, baseDurationDays: null, icon: null }],
    history: { progress: [], segments: [{ firstHour: 0, lastHour: 1000, order: { completed: [{ ordinal: 0, token: 'GER_rhineland', completedByHour: 50 }, { ordinal: 1, token: 'GER_old', completedByHour: null }], current: null, continuous: null, paused: false } }] } } as unknown as FocusReply;
  const badges = stabilityMarkers({ ...base, ideas, focus }, ALL);
  // The branch-start law and the one after an unrecorded hour are not changes.
  assert.deepEqual(badges.map(badge => [badge.hour, badge.glyph, badge.label]), [
    [50, '★', 'Focus: Rhineland (completed by then)'],
    [500, '⚙', 'Law: War Economy (taken on by then)'],
    [700, '⚙', 'Advisor: Albert Speer (appointed by then)'],
  ]);
});

test('a continuing advisor, a shared founding and a repeated focus are read honestly', () => {
  const shared: CountryGroupReply = { ...groups, factions: [{ id: '3', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 1000, nameReason: null, names: [{ firstHour: 100, lastHour: 1000, name: 'Pact', reason: null }],
    members: [{ tag: 'GER', firstHour: 100, lastHour: 1000 }, { tag: 'ITA', firstHour: 100, lastHour: 1000 }], leaders: [], incomplete: [] }], warStarts: [], capitulations: [], wars: [] };
  assert.deepEqual(stabilityMarkers({ ...base, groups: shared }, ALL).map(badge => badge.label), ['Joined Pact']);
  // A sole first member of an incomplete first roster (another member not resolved) only joined.
  const partial: CountryGroupReply = { ...shared, factions: [{ ...shared.factions[0], members: [{ tag: 'GER', firstHour: 100, lastHour: 1000 }], leaders: [], incomplete: [] }],
    unobserved: { humans: [], factions: [{ firstHour: 100, lastHour: 100 }], wars: [] } };
  assert.deepEqual(stabilityMarkers({ ...base, groups: partial }, ALL).map(badge => badge.label), ['Joined Pact']);
  // The same when only this faction's roster was incomplete then.
  const partialOwn: CountryGroupReply = { ...shared, factions: [{ ...shared.factions[0], members: [{ tag: 'GER', firstHour: 100, lastHour: 1000 }], leaders: [], incomplete: [{ firstHour: 100, lastHour: 100 }] }] };
  assert.deepEqual(stabilityMarkers({ ...base, groups: partialOwn }, ALL).map(badge => badge.label), ['Joined Pact']);
  // A country whose id changes tag while in the faction neither leaves nor joins it.
  const renamed: CountryGroupReply = { ...shared, identityChanges: [{ hour: 500, from: 'AAA', to: 'GER' }],
    factions: [{ id: '3', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 1000, nameReason: null, names: [{ firstHour: 100, lastHour: 1000, name: 'Pact', reason: null }], members: [{ tag: 'AAA', firstHour: 100, lastHour: 499 }, { tag: 'GER', firstHour: 500, lastHour: 1000 }], leaders: [], incomplete: [] }] };
  assert.deepEqual(stabilityMarkers({ ...base, groups: renamed }, ALL), []);
  // AAA founded it (its sole first member) and never left: the change of tag is no leave.
  assert.deepEqual(stabilityMarkers({ ...base, tag: 'AAA', groups: renamed }, ALL).map(badge => badge.label), ['Founded Pact']);
  // A tag change in the same hour as a real move between factions hides neither.
  const switched: CountryGroupReply = { ...shared, identityChanges: [{ hour: 500, from: 'AAA', to: 'GER' }], factions: [
    { id: '3', name: 'First Pact', recordedName: 'First Pact', firstHour: 100, lastHour: 1000, nameReason: null, names: [{ firstHour: 100, lastHour: 1000, name: 'First Pact', reason: null }], members: [{ tag: 'AAA', firstHour: 100, lastHour: 499 }, { tag: 'ITA', firstHour: 100, lastHour: 1000 }], leaders: [], incomplete: [] },
    { id: '4', name: 'Second Pact', recordedName: 'Second Pact', firstHour: 100, lastHour: 1000, nameReason: null, names: [{ firstHour: 100, lastHour: 1000, name: 'Second Pact', reason: null }], members: [{ tag: 'GER', firstHour: 500, lastHour: 1000 }, { tag: 'HUN', firstHour: 100, lastHour: 1000 }], leaders: [], incomplete: [] }] };
  assert.deepEqual(stabilityMarkers({ ...base, groups: switched }, ALL).map(badge => badge.label), ['Joined Second Pact']);
  assert.deepEqual(stabilityMarkers({ ...base, tag: 'AAA', groups: switched }, ALL).map(badge => badge.label), ['Joined First Pact', 'Left First Pact']);
  const ideas = {
    branchId: '1', tag: 'GER', warnings: [], unobserved: [], identityStarts: [], modifiers: [], texts: [], artwork: [], holders: [],
    spans: [
      { token: 'speer', firstHour: 100, lastHour: 199, slot: 'political_advisor', character: { id: 7, token: '', name: '' } },
      { token: 'speer', firstHour: 200, lastHour: 300, slot: 'political_advisor', character: { id: 7, token: 'GER_albert_speer', name: 'Albert Speer' } },
    ],
    ideas: [], categories: [{ token: 'government', name: null, kind: null, hidden: false, politicsTab: true, slots: [
      { token: 'political_advisor', name: null, character: true, seats: 3, laws: [] }] }],
  } as unknown as IdeaHistoryReply;
  // Appointed at 100 (not identified then); its details arriving at 200 is no new appointment.
  assert.deepEqual(stabilityMarkers({ ...base, ideas }, ALL).map(badge => [badge.hour, badge.label]), [[100, 'Advisor: holder not recorded (appointed by then)']]);
  // A holder is matched by script token too: a later identified holder never names an earlier unidentified one.
  const unnamed = { ...ideas, spans: [
    { token: 'speer', firstHour: 100, lastHour: 300, slot: 'political_advisor', character: { id: 7, token: '', name: '' } }],
  holders: [{ token: 'speer', character: 7, characterToken: 'GER_albert_speer', name: '', artwork: { token: 'speer', name: 'Albert Speer', description: null, baseDurationDays: null, icon: null } }] } as unknown as IdeaHistoryReply;
  assert.deepEqual(stabilityMarkers({ ...base, ideas: unnamed }, ALL).map(badge => badge.label), ['Advisor: holder not recorded (appointed by then)']);
  const focus = { warnings: [], artwork: [], history: { progress: [], segments: [
    { firstHour: 0, lastHour: 150, order: { completed: [{ ordinal: 0, token: 'repeatable', completedByHour: 100 }], current: null, continuous: null, paused: false } },
    { firstHour: 151, lastHour: 1000, order: { completed: [{ ordinal: 0, token: 'repeatable', completedByHour: 100 }, { ordinal: 1, token: 'repeatable', completedByHour: 200 }], current: null, continuous: null, paused: false } },
  ] } } as unknown as FocusReply;
  assert.deepEqual(stabilityMarkers({ ...base, extent: [150, 1000], focus }, ALL).map(badge => [badge.hour, badge.label]), [[200, 'Focus: Focus name unavailable (completed by then)']]);
  // Two completions seen at one hour after a gap: both kept, each marked as a bound.
  const twice = { warnings: [], artwork: [], history: { progress: [], segments: [
    { firstHour: 0, lastHour: 99, order: { completed: [], current: null, continuous: null, paused: false } },
    { firstHour: 300, lastHour: 1000, order: { completed: [{ ordinal: 0, token: 'repeatable', completedByHour: 300 }, { ordinal: 1, token: 'repeatable', completedByHour: 300 }], current: null, continuous: null, paused: false } },
  ] } } as unknown as FocusReply;
  assert.deepEqual(stabilityMarkers({ ...base, focus: twice }, ALL).map(badge => badge.label), ['Focus: Focus name unavailable (completed by then)', 'Focus: Focus name unavailable (completed by then)']);
  // A law continuing across a change of country identity is no new law.
  const kept = { ...ideas, holders: [], spans: [
    { token: 'law', firstHour: 0, lastHour: 149, slot: 'economy', character: null }, { token: 'law', firstHour: 150, lastHour: 300, slot: 'economy', character: null }],
    categories: [{ token: 'government', name: null, kind: null, hidden: false, politicsTab: true, slots: [{ token: 'economy', name: null, character: false, seats: 1, laws: ['law'] }] }] } as unknown as IdeaHistoryReply;
  assert.deepEqual(stabilityMarkers({ ...base, ideas: kept }, ALL), []);
  // A new country identity's initial law is its starting state, not a change.
  const replaced = { ...kept, spans: [
    { token: 'other', firstHour: 0, lastHour: 99, slot: 'economy', character: null }, { token: 'law', firstHour: 100, lastHour: 300, slot: 'economy', character: null }],
    identityStarts: [0, 100] } as unknown as IdeaHistoryReply;
  assert.deepEqual(stabilityMarkers({ ...base, ideas: replaced }, ALL), []);
  assert.equal(stabilityMarkers({ ...base, ideas: { ...replaced, identityStarts: [0] } }, ALL).length, 1);
  // The same character moving to another role in its seat is no new appointment.
  const moved = { ...ideas, spans: [
    { token: 'role_a', firstHour: 100, lastHour: 199, slot: 'political_advisor', character: { id: 7, token: 'GER_a', name: 'GER_a' } },
    { token: 'role_b', firstHour: 200, lastHour: 300, slot: 'political_advisor', character: { id: 7, token: 'GER_a', name: 'GER_a' } }] } as unknown as IdeaHistoryReply;
  assert.deepEqual(stabilityMarkers({ ...base, ideas: moved }, ALL).map(badge => badge.hour), [100]);
  // A designer in a law slot is not a law.
  const designer = { ...kept, spans: [{ token: 'designer', firstHour: 150, lastHour: 300, slot: 'economy', character: null }],
    ideas: [{ token: 'designer', category: 'government', slot: 'economy', law: false, designer: true, effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null }] } as unknown as IdeaHistoryReply;
  assert.deepEqual(stabilityMarkers({ ...base, ideas: designer }, ALL), []);
});

test('an enemy capitulation after an unrecorded war hour, and events outside the group, are left out', () => {
  // The war with Norway ended at 600; hour 601 went unrecorded, so a capitulation then is not known to be an enemy's.
  const gap: CountryGroupReply = { ...groups, factions: [], warStarts: [], wars: [{ first: 'GER', second: 'NOR', firstHour: 500, lastHour: 600 }, { first: 'GER', second: 'DEN', firstHour: 500, lastHour: 600 }],
    capitulations: [{ hour: 601, tag: 'NOR' }, { hour: 700, tag: 'DEN' }, { hour: 601, tag: 'DEN' }], unobserved: { humans: [], factions: [], wars: [{ firstHour: 601, lastHour: 601 }] } };
  assert.deepEqual(stabilityMarkers({ ...base, groups: gap }, ALL), []);
  const seen: CountryGroupReply = { ...gap, unobserved: { humans: [], factions: [], wars: [] } };
  assert.deepEqual(stabilityMarkers({ ...base, groups: seen }, ALL).map(badge => badge.hour), [601, 601]);
  // In a group only while a member: the war at 180 is outside hours 190-1000.
  const member = (hour: number) => hour >= 190;
  assert.ok(!stabilityMarkers({ ...base, groups, member }, ALL).some(badge => badge.hour < 190));
  assert.ok(stabilityMarkers({ ...base, groups }, ALL).some(badge => badge.hour === 180));
});

test('other countries are named as they were at each event hour', () => {
  const asked: string[] = [];
  const labels = stabilityMarkers({ ...base, groups, countryName: (tag, hour) => { asked.push(`${tag}@${hour}`); return `${tag}${hour}`; } }, new Set(['wars'] as const)).map(badge => badge.label);
  assert.ok(labels.includes('War with POL180 began'));
  assert.ok(asked.includes('POL@180') && asked.includes('FRA@450'));
});

test('only events shown ask for names', () => {
  const asked: number[] = [];
  stabilityMarkers({ ...base, extent: [190, 1000], groups, countryName: (tag, hour) => { asked.push(hour); return tag; } }, new Set(['wars'] as const));
  assert.ok(!asked.includes(180));
});

test('an enemy that changes tag the hour it capitulates still counts', () => {
  const renamed: CountryGroupReply = { ...groups, factions: [], warStarts: [], wars: [{ first: 'AAA', second: 'GER', firstHour: 100, lastHour: 199 }],
    capitulations: [{ hour: 200, tag: 'BBB' }], identityChanges: [{ hour: 200, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [] } };
  assert.deepEqual(stabilityMarkers({ ...base, groups: renamed }, ALL).map(badge => badge.label), ['BBB capitulated']);
  assert.deepEqual(stabilityMarkers({ ...base, groups: { ...renamed, identityChanges: [] } }, ALL), []);
});

test('faction events use dated names and display overrides without backfilling gaps', () => {
  const faction = { ...groups.factions[1], firstHour: 100, lastHour: 499, name: 'Later pact', names: [
    { firstHour: 100, lastHour: 299, name: 'Opening pact', reason: null },
    { firstHour: 300, lastHour: 499, name: 'Later pact', reason: null },
  ], members: [{ tag: 'GER', firstHour: 150, lastHour: 299 }], leaders: [], incomplete: [] };
  const data = { ...groups, factions: [faction], unobserved: { humans: [], factions: [], wars: [] } };
  const events = (data: CountryGroupReply) => stabilityMarkers({ ...base, groups: data }, new Set<MarkerKind>(['factions']));
  assert.deepEqual(events(data).map(badge => badge.label), ['Joined Opening pact', 'Left Later pact']);
  const gap = { ...data, factions: [{ ...faction, names: [faction.names[0], { ...faction.names[1], firstHour: 301 }] }] };
  assert.deepEqual(events(gap).map(badge => badge.label), ['Joined Opening pact', 'Left Unnamed faction (Not recorded)']);
  // A dissolved faction has no read at the leaving hour; identify it from its final membership read.
  const dissolved = { ...data, factions: [{ ...faction, lastHour: 299, names: [faction.names[0]] }] };
  assert.deepEqual(events(dissolved).map(badge => badge.label), ['Joined Opening pact', 'Left Opening pact']);
  const custom = stabilityMarkers({ ...base, groups: gap, factionName: () => ({ name: 'My pact', reason: null }) }, new Set<MarkerKind>(['factions']));
  assert.deepEqual(custom.map(badge => badge.label), ['Joined My pact', 'Left My pact']);
});
