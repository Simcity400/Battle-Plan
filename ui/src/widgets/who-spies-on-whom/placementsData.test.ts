import { test } from 'node:test';
import assert from 'node:assert/strict';
import { countryGroupReply } from '../../dashboard/countryGroups.fixture.ts';
import type { OperativePlacementReply } from '../../generated/dashboard';
import { countryNodeId, placementChord, placementChordMemo, placementTooltip, placementsAt, OTHER_ID, rankedCountries, placementTotals, selectPlacements, validatePlacementsReply } from './placementsData.ts';
import { tooltipText } from '../../charts/tooltip.ts';
import { createWidget } from '../../dashboard/model.ts';
import '../definitions.ts';

const reply = (): OperativePlacementReply => ({ branchId: '7', warnings: [],
  pairs: [{ id: 'pair', owner: 'Mod | spy', target: 'renamed target', runs: [
    { firstHour: 10, lastHour: 12, count: 2, operatives: [{ id: '1', name: 'Modded Alice', unnamed: false }, { id: '2', name: 'Renamed operative', unnamed: false }] }, { firstHour: 15, lastHour: 17, count: 1, operatives: [{ id: '1', name: 'Modded Alice', unnamed: false }] },
  ] }], periods: [{ firstHour: 10, lastHour: 12, availability: 'recorded' }, { firstHour: 13, lastHour: 14, availability: 'tablesNotRecorded' },
    { firstHour: 15, lastHour: 17, availability: 'recorded' }], readHours: [10, 12, 15] });
const labels = (tag: string) => ({ name: `Recorded ${tag}`, flag: `flag:${tag}` });

test('manual placement selection keeps earlier aliases and every holder, without parsing opaque pair IDs', () => {
  const groups = countryGroupReply({ countryIdentities: [
    { tag: 'X', identities: [{ firstHour: 10, lastHour: 11, slot: 2, countryId: 20 }, { firstHour: 13, lastHour: 14, slot: 1, countryId: 10 }] },
    { tag: 'Y', identities: [{ firstHour: 10, lastHour: 11, slot: 1, countryId: 10 }, { firstHour: 13, lastHour: 14, slot: 3, countryId: 30 }] },
    { tag: 'B_OTHER', identities: [{ firstHour: 13, lastHour: 14, slot: 2, countryId: 20 }] },
  ] });
  const selection: import('../../dashboard/model').CountrySelection = { kind: 'tags', tags: ['X'] };
  const placement = (id: string, owner: string, target: string) => ({ id, owner, target, count: 1, operatives: [{ id, name: null, unnamed: true }] });
  const before = [placement('opaque A', 'Y', 'T'), placement('opaque B', 'X', 'T')];
  assert.equal(selectPlacements(before, selection, groups, 10, 14).placements.length, 2);
  assert.equal(selectPlacements(before, selection, groups, 12, 14).placements.length, 0);
  const after = [placement('opaque A', 'X', 'T'), placement('opaque B', 'B_OTHER', 'T'),
    placement('[10,20]', 'Y', 'T'), placement('opaque reverse', 'Y', 'X')];
  assert.deepEqual(selectPlacements(after, selection, groups, 13, 14).placements.map(pair => pair.id), ['opaque A', 'opaque B', 'opaque reverse']);
});

test('native placement filtering retains the final holder in a 5000-country catalogue', () => {
  const groups = countryGroupReply({ countryIdentities: Array.from({ length: 5000 }, (_, index) => ({ tag: `Country ${index}`,
    identities: [{ firstHour: 10, lastHour: 17, slot: index + 1, countryId: index + 10001 }] })) });
  const placements = groups.countryIdentities.map((country, index) => ({ id: `opaque ${index}`, owner: country.tag, target: 'unrelated', count: 1,
    operatives: [{ id: String(index), name: null, unnamed: true }] }));
  assert.deepEqual(selectPlacements(placements, { kind: 'tags', tags: ['Country 4999'] }, groups, 10, 17).placements.map(pair => pair.id), ['opaque 4999']);
});
test('validates a modded opaque pair, retains counts and names and the later equal read', () => {
  const r = validatePlacementsReply(reply());
  assert.equal(placementsAt(r, 11).placements[0].count, 2);
  assert.equal(placementsAt(r, 12).read, 12);
  const chord = placementChord(placementsAt(r, 12).placements, 12, 12, labels, () => 'recorded-color', 'muted');
  const tip = placementTooltip({ kind: 'flow', id: chord.data.flows[0].id }, chord)!;
  assert.equal(tip.title, 'Recorded Mod | spy has 2 operatives in Recorded renamed target');
  assert.match(tooltipText(tip), /Modded Alice; Renamed operative$/);
});
test('a complete read holds through unread hours with its read date and daily note', () => {
  const r = validatePlacementsReply(reply());
  for (const hour of [10, 11]) {
    const current = placementsAt(r, hour);
    assert.equal(current.period?.availability, 'recorded');
    assert.equal(current.read, 10);
    assert.equal(current.placements[0].count, 2);
    const chord = placementChord(current.placements, 12, current.read!, labels, () => 'recorded-color', 'muted');
    const tip = placementTooltip({ kind: 'flow', id: chord.data.flows[0].id }, chord)!;
    assert.equal(tip.note, undefined);
    assert.match(tooltipText(tip), /Modded Alice; Renamed operative$/);
  }
  assert.equal(placementsAt(r, 17).read, 15);
});
test('a held keyframe in the middle preserves the hold without supplying a new read', () => {
  const r = reply();
  r.readHours = [10, 15];
  const validated = validatePlacementsReply(r);
  const before = placementsAt(validated, 10), heldKeyframe = placementsAt(validated, 11), after = placementsAt(validated, 12);
  assert.equal(heldKeyframe.read, 10);
  assert.equal(after.read, 10);
  assert.deepEqual(heldKeyframe.placements, before.placements);
  assert.deepEqual(after.placements, before.placements);
});
test('a gap ends a hold and the next stretch stays unavailable until a complete read', () => {
  const r = reply();
  r.periods[1].firstHour = 14;
  r.readHours = [10, 15];
  const validated = validatePlacementsReply(r);
  assert.equal(placementsAt(validated, 12).read, 10);
  assert.deepEqual(placementsAt(validated, 13), { period: null, read: null, placements: [] });
  const beforeRead = placementsAt(validated, 14);
  assert.equal(beforeRead.period?.availability, 'tablesNotRecorded');
  assert.equal(beforeRead.read, null);
  assert.deepEqual(beforeRead.placements, []);
  assert.equal(placementsAt(validated, 15).read, 15);
});
test('a failed membership read ends the hold until the next complete read', () => {
  const r = validatePlacementsReply(reply());
  assert.equal(placementsAt(r, 12).read, 12);
  for (const hour of [13, 14]) {
    const current = placementsAt(r, hour);
    assert.equal(current.period?.availability, 'tablesNotRecorded');
    assert.equal(current.read, null);
    assert.deepEqual(current.placements, []);
  }
  assert.equal(placementsAt(r, 15).read, 15);
});
test('a complete empty read replaces a placement hold and remains known empty', () => {
  const r = reply();
  r.periods = [{ firstHour: 10, lastHour: 17, availability: 'recorded' }];
  r.readHours = [10, 13, 15];
  const validated = validatePlacementsReply(r);
  assert.equal(placementsAt(validated, 12).read, 10);
  for (const hour of [13, 14]) {
    const current = placementsAt(validated, hour);
    assert.equal(current.period?.availability, 'recorded');
    assert.equal(current.read, 13);
    assert.deepEqual(current.placements, []);
  }
  assert.equal(placementsAt(validated, 15).placements[0].count, 1);
});
test('a disabled intelligence feature ends the hold before a later complete read', () => {
  const r = reply();
  r.periods[1].availability = 'featureAbsent';
  const validated = validatePlacementsReply(r);
  const disabled = placementsAt(validated, 13);
  assert.equal(disabled.period?.availability, 'featureAbsent');
  assert.equal(disabled.read, null);
  assert.deepEqual(disabled.placements, []);
  assert.equal(placementsAt(validated, 15).read, 15);
});
test('the first hour before any complete read is unavailable and never backfilled', () => {
  const r = reply();
  r.periods.unshift({ firstHour: 8, lastHour: 9, availability: 'tablesNotRecorded' });
  const validated = validatePlacementsReply(r);
  for (const hour of [8, 9]) {
    const current = placementsAt(validated, hour);
    assert.equal(current.period?.availability, 'tablesNotRecorded');
    assert.equal(current.read, null);
    assert.deepEqual(current.placements, []);
  }
  assert.equal(placementsAt(validated, 10).read, 10);
});
test('unavailable, gap, known count and complete empty remain distinct', () => {
  const r = reply();
  assert.equal(placementsAt(r, 9).period, null);
  assert.equal(placementsAt(r, 13).period?.availability, 'tablesNotRecorded');
  assert.equal(placementsAt(r, 15).placements[0].count, 1);
  r.pairs = [];
  assert.equal(placementsAt(r, 15).period?.availability, 'recorded');
  assert.equal(placementsAt(r, 15).placements.length, 0);
  r.periods[2].availability = 'featureAbsent'; r.readHours = [10, 12];
  assert.equal(validatePlacementsReply(r).periods[2].availability, 'featureAbsent');
});
test('rejects overlapping runs, unsourced changes, invalid rosters and coverage in gaps', () => {
  const invalid: unknown[] = [null, {}, { ...reply(), pairs: null }, { ...reply(), readHours: [10, 10] }];
  for (const count of ['2', -1, 0, 1.5, 4294967296]) {
    const r = reply(); Object.assign(r.pairs[0].runs[0], { count }); invalid.push(r);
  }
  const overlap = reply(); overlap.pairs[0].runs[1].firstHour = 12; invalid.push(overlap);
  const gap = reply(); gap.pairs[0].runs[0].lastHour = 14; invalid.push(gap);
  const noRead = reply(); noRead.readHours = [12, 15, 17]; invalid.push(noRead);
  const unreadStart = reply(); unreadStart.pairs[0].runs[0].firstHour = 11; invalid.push(unreadStart);
  const unreadEnd = reply(); unreadEnd.readHours = [10, 15]; unreadEnd.pairs[0].runs[0].lastHour = 11; invalid.push(unreadEnd);
  const unavailableRead = reply(); unavailableRead.readHours = [10, 12, 13, 15]; invalid.push(unavailableRead);
  const gapRead = reply(); gapRead.periods[1].firstHour = 14; gapRead.readHours = [10, 12, 13, 15]; invalid.push(gapRead);
  const unreadableEmpty = reply(); unreadableEmpty.pairs[0].runs[0].operatives[0].name = ''; invalid.push(unreadableEmpty);
  const namedUnnamed = reply(); namedUnnamed.pairs[0].runs[0].operatives[0].unnamed = true; invalid.push(namedUnnamed);
  const missingUnnamed = reply(); Object.assign(missingUnnamed.pairs[0].runs[0].operatives[0], { unnamed: undefined }); invalid.push(missingUnnamed);
  const unknownAvailability = reply(); Object.assign(unknownAvailability.periods[1], { availability: 'unsupported' }); invalid.push(unknownAvailability);
  const duplicate = reply(); duplicate.pairs.push(duplicate.pairs[0]); invalid.push(duplicate);
  const aliasDuplicate = reply(); aliasDuplicate.pairs.push({ ...aliasDuplicate.pairs[0], id: 'another identity' }); invalid.push(aliasDuplicate);
  const duplicateOperative = reply(); duplicateOperative.pairs[0].runs[0].operatives[1].id = '1'; invalid.push(duplicateOperative);
  const shortRoster = reply(); shortRoster.pairs[0].runs[0].operatives.pop(); invalid.push(shortRoster);
  const twoTargets = reply(); twoTargets.pairs.push({ ...twoTargets.pairs[0], id: 'another', target: 'third' }); invalid.push(twoTargets);
  for (const r of invalid) assert.throws(() => validatePlacementsReply(r), /malformed/);
});
test('all is the new widget default; explicit choices include either endpoint', () => {
  assert.deepEqual(createWidget('who-spies-on-whom', 'track').countries, { kind: 'all' });
  const placements = placementsAt(reply(), 10).placements;
  assert.equal(selectPlacements(placements, { kind: 'all' }, undefined, 10, 17).placements.length, 1);
  const groups = countryGroupReply({ countryIdentities: [{ tag: 'renamed target', identities: [{ firstHour: 10, lastHour: 17, slot: 1, countryId: 7 }] }] });
  assert.match(selectPlacements(placements, { kind: 'tags', tags: ['renamed target'] }, undefined, 10, 17).reason!, /Loading countries/);
  assert.equal(selectPlacements(placements, { kind: 'tags', tags: ['renamed target'] }, groups, 10, 17).placements.length, 1);
  assert.equal(selectPlacements(placements, { kind: 'tags', tags: ['absent'] }, undefined, 10, 17).placements.length, 0);
});
test('a smart group resolves at its chosen hour and missing membership is unavailable', () => {
  const placements = placementsAt(reply(), 10).placements;
  const groups = countryGroupReply({ countryIdentities: [{ tag: 'renamed target', identities: [{ firstHour: 10, lastHour: 17, slot: 1, countryId: 7 }] }], humans: [{ tag: 'renamed target', firstHour: 10, lastHour: 11 }], unobserved: { humans: [{ firstHour: 12, lastHour: 12 }], factions: [], wars: [] } });
  const selection = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
  assert.equal(selectPlacements(placements, selection, groups, 10, 17).placements.length, 1);
  assert.match(selectPlacements(placements, selection, groups, 12, 17).reason!, /Group membership is unavailable at this time/);
  assert.equal(selectPlacements(placements, selection, undefined, 12, 17).reason, 'Loading countries…');
});
test('folding is lossless, preserves acting colours and names every Other member', () => {
  const placements = [
    { id: 'a', owner: 'spy', target: 'target', count: 5, operatives: Array.from({ length: 5 }, (_, i) => ({ id: String(10 + i), name: `operative ${i}`, unnamed: false })) },
    { id: 'b', owner: 'third', target: 'fourth', count: 1, operatives: [{ id: '2', name: 'two', unnamed: false }] },
    { id: 'c', owner: 'fourth', target: 'third', count: 1, operatives: [{ id: '3', name: 'three', unnamed: false }] },
  ];
  const chord = placementChord(placements, 2, 10, labels, tag => `color:${tag}`, 'muted');
  assert.deepEqual(rankedCountries(placementTotals(placements)).slice(0, 2), ['spy', 'target']);
  assert.equal(chord.data.nodes.find(n => n.id === countryNodeId('spy'))?.color, 'color:spy');
  assert.equal(chord.data.flows.reduce((s, f) => s + f.value, 0), 7);
  const arc = placementTooltip({ kind: 'node', id: OTHER_ID }, chord)!;
  assert.match(tooltipText(arc), /Recorded third/); assert.match(tooltipText(arc), /Recorded fourth/);
  const self = chord.data.flows.find(f => f.source === OTHER_ID && f.target === OTHER_ID)!;
  const tip = placementTooltip({ kind: 'flow', id: self.id }, chord)!;
  assert.equal(tip.rows.at(-1)?.value, '2');
  assert.match(tooltipText(tip), /two/); assert.match(tooltipText(tip), /three/);
});
test('5,000 modded countries and 20,000 flows fold to the chosen twelve plus Other', () => {
  const placements = Array.from({ length: 20000 }, (_, i) => ({ id: String(i), owner: `mod ${i % 5000}`, target: `mod ${(i + 1) % 5000}`, count: 1, operatives: [{ id: String(i), name: null, unnamed: false }] }));
  const chord = placementChord(placements, 12, 10, labels, () => 'content-color', 'muted');
  assert.equal(chord.data.nodes.length, 13);
  assert.equal([...chord.flows.values()].reduce((sum, members) => sum + members.length, 0), 20000);
  assert.equal(chord.nodes.get(OTHER_ID)?.length, 4988);
});

test('playback memo reuses a 5,000-country / 20,000-flow fold until exact data changes', () => {
  const at = placementChordMemo();
  const placements = Array.from({ length: 20000 }, (_, i) => ({ id: String(i), owner: `mod ${i % 5000}`, target: `mod ${(i + 1) % 5000}`, count: 1, operatives: [{ id: String(i), name: null, unnamed: false }] }));
  const first = at(placements, 12, 10, labels, () => 'recorded color', 'muted');
  for (let hour = 11; hour < 34; hour++) {
    assert.equal(at(placements.map(n => ({ ...n })), 12, 10, labels, () => 'recorded color', 'muted').data, first.data);
  }
  const nextRead = at(placements, 12, 34, labels, () => 'recorded color', 'muted');
  assert.notEqual(nextRead.data, first.data, 'equal values at a later read update exact tooltip dates');
  assert.equal(nextRead.read, 34);
  const renamed = at(placements, 12, 34, tag => ({ ...labels(tag), name: `new ${tag}` }), () => 'recorded color', 'muted');
  assert.notEqual(renamed.data, nextRead.data);
  const recolored = at(placements, 12, 34, tag => ({ ...labels(tag), name: `new ${tag}` }), () => 'new color', 'muted');
  assert.notEqual(recolored.data, renamed.data);
  assert.notEqual(at(placements, 11, 34, labels, () => 'recorded color', 'muted').data, recolored.data);
  assert.notEqual(at(placements.slice(1), 11, 34, labels, () => 'recorded color', 'muted').data, recolored.data);
  const same = at(placements, 12, 10, labels, () => 'recorded color', 'muted');
  const changed = placements.map(n => ({ ...n, operatives: n.operatives.map(o => ({ ...o, name: 'New name' })) }));
  assert.notEqual(at(changed, 12, 10, labels, () => 'recorded color', 'muted').data, same.data, 'names change the exact tooltip even when counts stay equal');
  const missing = at(placements, 12, 10, labels, () => 'recorded color', 'muted');
  const unnamed = placements.map(n => ({ ...n, operatives: n.operatives.map(o => ({ ...o, unnamed: true })) }));
  assert.notEqual(at(unnamed, 12, 10, labels, () => 'recorded color', 'muted').data, missing.data, 'an unnamed state changes exact tooltip wording when both names are null');
});
test('country ranking ties and ring order follow opaque UTF-8 tag order', () => {
  const tags = ['\u{10000}', '\uE000', 'a', 'A', 'é'];
  const expected = ['A', 'a', 'é', '\uE000', '\u{10000}'];
  assert.deepEqual(rankedCountries(new Map(tags.map(tag => [tag, 2n]))), expected);
  const placements = tags.map((tag, i) => ({ id: String(i), owner: tag, target: tag, count: 1, operatives: [{ id: String(i), name: 'Named operative', unnamed: false }] }));
  const chord = placementChord(placements, tags.length, 10, labels, () => 'recorded', 'muted');
  assert.deepEqual([...chord.countries.values()], expected);
  assert.deepEqual(chord.data.nodes.map(node => node.id), expected.map(countryNodeId));
});

test('a home placement contributes to both arc totals and missing names stay visible', () => {
  const placements = [{ id: 'home', owner: 'modded home', target: 'modded home', count: 1, operatives: [{ id: '44', name: null, unnamed: false }] }];
  const chord = placementChord(placements, 12, 10, labels, () => 'recorded', 'muted');
  const tip = placementTooltip({ kind: 'node', id: countryNodeId('modded home') }, chord)!;
  assert.deepEqual(tip.rows.map(r => r.value), ['1', '1']);
  assert.match(tooltipText(placementTooltip({ kind: 'flow', id: chord.data.flows[0].id }, chord)!), /Unknown operative/);
});
test('the widget words an unnamed state while a modded literal stays visible unchanged', () => {
  const r = reply();
  r.pairs[0].runs[0].operatives = [{ id: '1', name: null, unnamed: true }, { id: '2', name: 'Modded | literal', unnamed: false }];
  const current = placementsAt(validatePlacementsReply(r), 10);
  const chord = placementChord(current.placements, 12, current.read!, labels, () => 'recorded', 'muted');
  const tip = tooltipText(placementTooltip({ kind: 'flow', id: chord.data.flows[0].id }, chord)!);
  assert.match(tip, /Unnamed operative; Modded \| literal/);
  assert.doesNotMatch(tip, /Name not recorded/);
});

test('hourly tag changes split a held membership without inventing a daily read', () => {
  const r = reply(), original = r.pairs[0];
  original.runs[0].lastHour = 10;
  r.pairs.push({ ...original, id: 'renamed identity presentation', target: 'renamed hourly target',
    runs: [{ ...original.runs[0], firstHour: 11, lastHour: 11 }] });
  assert.equal(validatePlacementsReply(r), r);
  assert.equal(placementsAt(r, 11).read, 10);
  assert.equal(placementsAt(r, 11).placements[0].target, 'renamed hourly target');
});

test('native chord flags retain pending and refusal changes without flagging the folded aggregate', () => {
  const placements = placementsAt(validatePlacementsReply(reply()), 12).placements;
  const memo = placementChordMemo();
  let pending = true, reason: string | undefined;
  const dated = (tag: string) => ({ name: tag, flagPending: pending, flagReason: reason });
  const first = memo(placements, 1, 12, dated, () => 'native-color', 'muted');
  assert.equal(first.data.nodes.find(node => node.id !== OTHER_ID)?.countryFlag?.flagPending, true);
  assert.equal(first.data.nodes.find(node => node.id === OTHER_ID)?.countryFlag, undefined);
  pending = false; reason = 'Synthetic flag definition unavailable in the recorded mod.';
  const completed = memo(placements, 1, 12, dated, () => 'native-color', 'muted');
  assert.notEqual(completed, first, 'a completed refusal must replace the pending canvas/tooltip state');
  assert.equal(completed.data.nodes.find(node => node.id !== OTHER_ID)?.countryFlag?.flagReason, reason);
  assert.equal(memo(placements, 1, 12, dated, () => 'native-color', 'muted'), completed);
  const other = placementTooltip({ kind: 'node', id: OTHER_ID }, completed)!;
  assert.equal(other.rows.at(-1)?.countryFlag?.flagReason, reason);
  reason = 'A different recorded refusal';
  assert.notEqual(memo(placements, 1, 12, dated, () => 'native-color', 'muted'), completed);
});
