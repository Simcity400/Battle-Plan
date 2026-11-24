import assert from 'node:assert/strict';
import test from 'node:test';
import type { CountryGroupReply, ShipsSunkManifestReply, ShipsSunkReply, ShipsSunkRow } from '../../generated/dashboard';
import { logCountryRequests } from '../../charts/log/logData.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { engineHour, formatDateTime } from '../../dashboard/calendar.ts';
import { countryMatch } from '../../dashboard/groups.ts';
import { manifestClassChoices, shipAction, shipClassChoices, shipClassName, shipCountryLabels, shipCountryRequests, shipCauseLabel, shipDescription, shipLogOrder, shipLogRows, shipTooltip,
  validateShipsSunkReply } from './shipsSunkData.ts';

function row(overrides: Partial<ShipsSunkRow> = {}): ShipsSunkRow {
  return { sinkHour: 102, knownAt: 110, name: 'Recorded vessel', classIndex: 0, recordedNameIndex: 0,
    victim: 'victim tag', attacker: 'mod attacker', victimAsOf: null, attackerAsOf: null, placeIndex: 0,
    cause: 'hostile', attributionName: null, assistedKill: false, ...overrides };
}
function fixture(): ShipsSunkReply {
  return { branchId: 'branch', window: 0, available: true, firstHour: 100, lastHour: 120, firstRead: 100,
    unread: [{ firstHour: 101, lastHour: 109 }], warnings: [], places: ['Renamed mod sea'], recordedNames: ['Recorded design'], classes: [
      { token: 999, key: 'mod/sea vessel', name: 'Extra mod ship class', icon: null },
      { token: 888, key: null, name: null, icon: null },
    ], rows: [row(), row({ sinkHour: 102 }), row({ sinkHour: 90, knownAt: 100 }),
      row({ sinkHour: 114, knownAt: 115, classIndex: 1, recordedNameIndex: null })] };
}

test('ships become visible at their supplying read while countries, ordering and ranges retain event hours', () => {
  const data = validateShipsSunkReply(fixture());
  const wanted: [string | null, number][] = [];
  const rows = shipLogRows(data, 105, [], 'victim', (tag, hour) => { wanted.push([tag, hour]); return true; }, null);
  assert.deepEqual(rows.map(item => [item.key, item.hour, item.by]), [['2', 90, undefined]]);
  assert.deepEqual(wanted, [['victim tag', 90]]);
  const recovered = shipLogRows(data, 110, [], 'victim', () => true, null);
  assert.deepEqual(recovered.map(item => [item.key, item.hour]), [['0', 102], ['1', 102], ['2', 90]]);
  assert.deepEqual(shipCountryRequests(recovered, data),
    [{ hour: 102, tag: 'victim tag' }, { hour: 102, tag: 'mod attacker' }, { hour: 90, tag: 'victim tag' }, { hour: 90, tag: 'mod attacker' }]);
  assert.equal(shipLogRows(data, 120, [], 'both', () => true, 103).length, 1);
  assert.equal(shipLogRows(data, 120, [], 'sinker', tag => tag === 'mod attacker', null).length, 4);
});

test('retained ships name and flag countries from the supplying read, with dated filters and tooltips', () => {
  const sink = engineHour({ year: 1943, month: 10, day: 13, hour: 8 })!;
  const read = engineHour({ year: 1943, month: 10, day: 24, hour: 16 })!;
  const data = fixture();
  data.firstHour = read; data.lastHour = read + 2; data.firstRead = read; data.unread = [];
  data.rows = [row({ sinkHour: sink, knownAt: read, victimAsOf: read, attackerAsOf: read }),
    row({ sinkHour: read + 1, knownAt: read + 2, victimAsOf: read + 2 })];
  const rows = shipLogRows(validateShipsSunkReply(data), read + 2, [], 'both', () => true, null);
  assert.deepEqual(rows.map(item => item.hour), [read + 1, sink]);
  assert.deepEqual(shipCountryRequests(rows, data), [
    { hour: read + 2, tag: 'victim tag' }, { hour: read + 1, tag: 'mod attacker' },
    { hour: read, tag: 'victim tag' }, { hour: read, tag: 'mod attacker' },
  ]);
  const labelAt = (hour: number) => (tag: string) => ({ name: `${tag} at ${hour}`, flag: `flag-${tag}-${hour}` });
  const retained = data.rows[0];
  const labels = shipCountryLabels(retained, labelAt);
  assert.deepEqual(labels.victim, labelAt(read)('victim tag'));
  assert.deepEqual(labels.attacker, labelAt(read)('mod attacker'));
  const mixed = shipCountryLabels(data.rows[1], labelAt);
  assert.deepEqual(mixed.victim, labelAt(read + 2)('victim tag'));
  assert.deepEqual(mixed.attacker, labelAt(read + 1)('mod attacker'));
  const groups: CountryGroupReply = { branchId: 'branch', warnings: [], identityChanges: [], factions: [], wars: [],
    countryIdentities: [{ tag: 'victim tag', identities: [{ firstHour: read, lastHour: read + 2, slot: 1, countryId: 10 }] }],
    warStarts: [], capitulations: [], unobserved: { humans: [], factions: [], wars: [] },
    humans: [{ tag: 'victim tag', firstHour: read, lastHour: read }] };
  const match = countryMatch({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, [], groups, data.lastHour);
  assert.deepEqual(shipLogRows(data, read + 2, [], 'victim', (tag, hour) => tag !== null && match.matches(tag, hour), null)
    .map(item => item.hour), [sink], 'membership uses the naming read, while the row retains its sinking date');
  const tip = shipTooltip(data, retained, labels.victim.name, labels.attacker.name);
  assert.equal(tip.subtitle, formatDateTime(sink));
  assert.equal(tip.rows?.find(item => item.label === 'Victim')?.value, `${labels.victim.name} (as of ${formatDateTime(read)})`);
  assert.equal(tip.rows?.find(item => item.label === 'Credited country')?.value, `${labels.attacker.name} (as of ${formatDateTime(read)})`);
  assert.doesNotMatch(String(shipTooltip(data, data.rows[1], 'Victim', 'Sinker').rows?.find(item => item.label === 'Credited country')?.value), /as of/);
  assert.equal(shipCountryLabels(row({ victim: null }), labelAt).victim.name, 'Not recorded');
  assert.equal(shipLogRows(data, read + 2, [], 'both', () => true, read).length, 1, 'range filtering still uses sinking dates');
});

test('modded class chips and sea zone are supplied by content; missing labels stay visible', () => {
  const data = fixture();
  data.classes.push({ token: 777, key: 'unused', name: 'Unused class', icon: null });
  const choices = shipClassChoices(data, 'campaign');
  assert.equal(choices.length, 2);
  assert.equal(choices.find(choice => choice.value === 'mod/sea vessel')?.label, 'Extra mod ship class');
  assert.equal(contentKey(setToken('mod/sea vessel')), 'mod/sea vessel');
  assert.equal(shipLogRows(data, 120, ['mod/sea vessel'], 'both', () => true, null).length, 3);
  assert.match(shipDescription(data, data.rows[0], 'Victim at event', 'Attacker at event'), /Extra mod ship class.*Renamed mod sea/);
  assert.doesNotMatch(shipDescription(data, data.rows[0], 'Victim', 'Attacker'), /Battle of|recorded reference/);
  data.classes[0].name = null;
  assert.equal(shipClassName(data, data.rows[0]), 'Recorded design');
  data.rows[0].recordedNameIndex = null;
  assert.equal(shipClassName(data, data.rows[0]), 'Class name unavailable');
  assert.equal(shipClassName(data, data.rows[3]), 'Class name unavailable');
  const unknownChoice = shipClassChoices(data, 'campaign').find(choice => choice.hint === 'Key: 888')!;
  assert.equal(shipLogRows(data, 120, [unknownChoice.value], 'both', () => true, null, shipLogOrder(data), 'campaign').length, 1);
  assert.equal(shipLogRows(data, 120, [unknownChoice.value], 'both', () => true, null, shipLogOrder(data), 'other campaign').length, 0);
  assert.equal(shipTooltip(data, data.rows[0], 'Victim', 'Attacker').rows?.some(item => item.label === 'Combat reference' || item.label === 'Province'), false);
});

test('class titles preserve authored empty and key-equal text while duplicate filter values stay in hints', () => {
  const data = fixture();
  data.classes[0].name = '';
  assert.equal(shipClassName(data, data.rows[0]), '');
  assert.equal(shipClassChoices(data, 'campaign').find(choice => choice.value === 'mod/sea vessel')?.label, '');
  data.classes[0].name = data.classes[0].key;
  assert.equal(shipClassName(data, data.rows[0]), 'mod/sea vessel');
  data.classes[0].name = null;
  data.recordedNames[0] = '';
  assert.equal(shipClassName(data, data.rows[0]), '', 'a known empty literal is independent of a missing content title');
  data.rows.forEach(row => { row.recordedNameIndex = null; });
  const choices = shipClassChoices(data, 'campaign');
  assert.deepEqual(choices.map(choice => choice.label), ['Class name unavailable', 'Class name unavailable']);
  assert.equal(new Set(choices.map(choice => choice.value)).size, 2);
  assert.deepEqual(choices.map(choice => choice.hint), ['Key: mod/sea vessel', 'Key: 888']);
  assert.equal(shipTooltip(data, data.rows[0], 'Victim', 'Attacker').rows.find(row => row.label === 'Class key')?.value, 'mod/sea vessel');
  assert.doesNotMatch(shipDescription(data, data.rows[0], 'Victim', 'Attacker'), /mod\/sea vessel/);
});

test('non-hostile and assisted flags never read as ordinary hostile kills', () => {
  const data = fixture();
  const cause = row({ cause: 'cause', attributionName: 'Recorded mine description', assistedKill: true });
  assert.equal(shipAction(cause, 'Attacker'), 'lost to Recorded mine description');
  assert.equal(shipCauseLabel(cause), 'Non-hostile cause');
  assert.doesNotMatch(shipDescription(data, cause, 'Victim', 'Attacker'), /sunk by/);
  assert.equal(shipTooltip(data, cause, 'Victim', 'Attacker').rows?.find(item => item.label === 'Assisted kill')?.value, 'Yes');
  for (const ordinary of [row(), row({ cause: 'assisted', assistedKill: true }), row({ cause: 'cause' })])
    assert.equal(shipTooltip(data, ordinary, 'Victim', 'Attacker').rows.find(item => item.label === 'Assisted kill'), undefined);
  assert.equal(shipAction(row({ cause: 'assisted', assistedKill: true }), 'Attacker'), 'sunk with assistance from Attacker');
  assert.equal(shipAction(row({ cause: 'hostile', assistedKill: true }), 'Attacker'), 'sunk with assistance from Attacker');
  assert.match(shipAction(row({ cause: 'cause' }), 'Attacker'), /Not recorded/);
});

test('malformed ship rows refuse while undrawable mod icons become absent artwork', () => {
  const data = fixture();
  data.classes[0].icon = 'http://invalid-art';
  assert.equal(validateShipsSunkReply(data).classes[0].icon, null);
  for (const change of [
    (reply: ShipsSunkReply) => { reply.rows[0].classIndex = 99; },
    (reply: ShipsSunkReply) => { reply.rows[0].sinkHour = 111; },
    (reply: ShipsSunkReply) => { reply.rows[0].placeIndex = 99; },
    (reply: ShipsSunkReply) => { reply.rows[0].recordedNameIndex = 99; },
    (reply: ShipsSunkReply) => { reply.rows[0].victimAsOf = 109; },
    (reply: ShipsSunkReply) => { reply.rows[0].attackerAsOf = 102; },
    (reply: ShipsSunkReply) => { reply.rows[0].victim = null; reply.rows[0].victimAsOf = 110; },
    (reply: ShipsSunkReply) => { reply.rows[0].knownAt = 121; },
  ]) {
    const reply = fixture(); change(reply);
    assert.throws(() => validateShipsSunkReply(reply), /malformed/);
  }
});

test('one token shares content and artwork while recorded class fallback names stay per row', () => {
  const data = fixture();
  data.classes[0].name = null;
  data.recordedNames.push('Another recorded design');
  data.rows[1].recordedNameIndex = 1;
  assert.equal(shipClassName(validateShipsSunkReply(data), data.rows[0]), 'Recorded design');
  assert.equal(shipClassName(data, data.rows[1]), 'Another recorded design');
  assert.equal(shipClassChoices(data, 'campaign').length, 2);
});

test('ships log retains fifty thousand sinking rows while only visible countries are requested', context => {
  const data = fixture();
  data.lastHour = 50100;
  data.rows = Array.from({ length: 50000 }, (_, index) => row({ sinkHour: 100 + index, knownAt: 100 + index,
    victim: `country ${index % 5000}`, attacker: `country ${(index + 1) % 5000}` }));
  const rows = shipLogRows(validateShipsSunkReply(data), data.lastHour, [], 'both', () => true, null);
  context.diagnostic(`50,000 ships / 5,000 countries UI fixture: ${Buffer.byteLength(JSON.stringify(data))} JSON bytes.`);
  assert.equal(rows.length, 50000);
  assert.equal(rows[0].hour, 50099);
  assert.equal(logCountryRequests(rows.slice(0, 20), item => ({ hour: item.hour, tags: [item.item.victim, item.item.attacker] })).length, 40);
  const ordered = shipLogOrder(data);
  const playbackRows = shipLogRows(data, 25099, [], 'both', () => true, null, ordered);
  assert.equal(playbackRows.length, 25000);
  assert.equal(playbackRows[0], ordered[25000]);
});



test('duplicate unknown class choices stay distinct without showing campaign tokens', () => {
  const data = fixture();
  data.classes.forEach(type => { type.name = null; type.key = null; });
  data.rows.forEach(row => { row.recordedNameIndex = null; });
  const choices = shipClassChoices(data, 'private-campaign');
  assert.equal(new Set(choices.map(choice => choice.value)).size, choices.length);
  assert.deepEqual(choices.map(choice => choice.label), ['Class name unavailable', 'Class name unavailable']);
  assert.deepEqual(choices.map(choice => choice.hint), ['Key: 999', 'Key: 888']);
  assert.ok(choices.every(choice => !choice.label.includes('private-campaign')));
});


test('the class choice covers the manifest classes, named or keyed, without campaign tokens in labels', () => {
  const manifest: ShipsSunkManifestReply = { branchId: '1', windows: [], available: true, firstRead: 100, recordedNames: [], warnings: [],
    classes: [{ token: 7, key: 'mod_frigate', name: 'Frigate', icon: null }, { token: 8, key: null, name: null, icon: null },
      { token: 9, key: 'mod_frigate', name: 'Frigate', icon: null }] };
  const choices = manifestClassChoices(manifest, 'private-campaign');
  assert.deepEqual(choices.map(choice => choice.label), ['Class name unavailable', 'Frigate']);
  assert.equal(choices[1].value, 'mod_frigate');
  assert.ok(choices.every(choice => !choice.label.includes('private-campaign')));
  assert.deepEqual(manifestClassChoices(undefined, 'x'), []);
});
