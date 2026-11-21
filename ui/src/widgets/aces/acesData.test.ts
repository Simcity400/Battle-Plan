import test from 'node:test';
import assert from 'node:assert/strict';
import { aceDate, aceJump, aceName, aceNameValue, aceRows, aceStatus, aceTooltip, aircraftName, validateAces, validatePortraits, wingName } from './acesData.ts';
import { DEFAULT_SORT, OPTIONS, definition } from './definition.ts';
import { sortRows } from '../../charts/table/logic.ts';
import { countryMatch } from '../../dashboard/groups.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { AirAce, AirAcesReply, CountryGroupReply } from '../../generated/dashboard';

const alive: AirAce = { id: '1', tag: 'opaque tag:Ω', name: 'Zed', callsign: 'Storm', wing: null, wingName: null, aircraft: [], alive: true, firstSeen: 100, death: null };
const dead: AirAce = { ...alive, id: '2', name: 'Alpha', alive: false, firstSeen: 90,
  death: { lastAlive: 100, byHour: 124, killerAce: '3', killerName: 'Killer then', killerTag: 'other tag' } };
const firstSeenDead: AirAce = { ...dead, id: '3', tag: 'other mod country', name: null, firstSeen: 124,
  death: { lastAlive: null, byHour: null, killerAce: null, killerName: null, killerTag: null } };
const reply = (aces: AirAce[] = [alive, dead, firstSeenDead]): AirAcesReply => ({ readHour: 124, missing: null, aces, warnings: [] });

test('an ace is dated by bounds: became an ace by its first sighting, killed by its first dead read, unknown when first seen dead', () => {
  const rows = aceRows(validateAces(reply()), 124);
  assert.match(aceStatus(rows[0]), /^Ace by /);
  assert.match(aceStatus(rows[1]), /^Killed by /);
  assert.equal(aceStatus(rows[2]), 'Killed, date not recorded');
  assert.equal(aceJump(rows[1]), 124);
  assert.equal(aceJump(rows[0]), 100);
  const tip = aceTooltip(rows[1], tag => `At read: ${tag}`);
  assert.equal(tip.title, 'Alpha “Storm”');
  assert.equal(tip.rows[0].label, 'At read: opaque tag:Ω');
  assert.equal(tip.rows.find(row => row.label === 'Killer ace')?.value, 'Killer then');
  assert.match(tip.rows.find(row => row.label === 'Death interval')?.value ?? '', /After .*; by /);
  assert.equal(aceTooltip(rows[2], String).rows.find(row => row.label === 'Killer country')?.value, 'Not recorded');
  assert.equal(aceName(rows[2]), 'Unknown ace', 'an unknown name never shows its id');
  const unseen = aceRows(reply([{ ...alive, firstSeen: null }]), 124)[0];
  assert.equal(aceStatus(unseen), 'Alive; first sighting not recorded');
  assert.equal(aceTooltip(unseen, String).rows.find(row => row.label === 'Became an ace')?.value, 'Not recorded');
});

test('a wing and its aircraft not recorded, absent and empty stay apart', () => {
  const row = (ace: Partial<AirAce>) => aceRows(reply([{ ...alive, ...ace }]), 124)[0];
  assert.equal(aircraftName(row({ wing: '7', aircraft: [{ id: '80', name: 'First design', missing: null }, { id: '81', name: null, missing: 'Aircraft 81 name unsupported' }] })), 'First design · Unknown aircraft');
  assert.equal(aircraftName(row({ wing: '7', aircraft: null })), 'Aircraft not recorded');
  assert.equal(aircraftName(row({ wing: '7', aircraft: [] })), 'No aircraft recorded');
  assert.equal(wingName(row({ wing: '7', wingName: null })), 'Wing name not recorded');
  assert.equal(wingName(row({ wing: '7', wingName: '' })), '', 'an empty stored name is a name');
  assert.equal(wingName(row({ wing: null })), 'No wing');
  assert.equal(aircraftName(row({ wing: null })), '');
});

test('chosen owners and groups are matched at the read the roster holds; an unknown owner only when every country is shown', () => {
  const owner = alive.tag!;
  const groups: CountryGroupReply = {
    branchId: '1', warnings: [], humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [owner, 'other mod country'].map((tag, slot) => ({ tag, identities: [{ firstHour: 100, lastHour: 147, slot, countryId: slot + 1 }] })),
    unobserved: { humans: [], factions: [], wars: [] },
  };
  groups.humans = [{ tag: owner, firstHour: 100, lastHour: 100 }, { tag: 'other mod country', firstHour: 101, lastHour: 147 }];
  const data = reply([alive, firstSeenDead, { ...dead, id: '4', tag: null }]);
  const keys = (selection: CountrySelection, read = 124, recorded: CountryGroupReply | null = groups) => {
    const match = countryMatch(selection, [owner], recorded ?? undefined, 147);
    return match.ready ? aceRows(data, read, (tag, at) => tag === null ? selection.kind === 'all' : match.matches(tag, at)).map(row => row.key) : [];
  };
  assert.deepEqual(keys({ kind: 'all' }), ['1', '3', '4']);
  assert.deepEqual(keys({ kind: 'selected' }), ['1']);
  assert.deepEqual(keys({ kind: 'tags', tags: ['other mod country'] }), ['3']);
  const humans = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
  assert.deepEqual(keys(humans, 100), ['1'], 'the read hour, not the playhead, decides membership');
  assert.deepEqual(keys(humans, 124), ['3']);
  assert.deepEqual(keys(humans, 124, null), [], 'pending membership does not show every country');
  const nobody = { ...groups, humans: [] };
  assert.equal(countryMatch(humans, [owner], nobody, 147).reason, 'This group has no members in this campaign.');
  assert.equal(countryMatch(humans, [owner], { ...nobody, unobserved: { ...nobody.unobserved, humans: [{ firstHour: 120, lastHour: 130 }] } }, 147).reason,
    'Group membership is incomplete for this campaign.');
  assert.equal(countryMatch(humans, [owner], groups, 147).reason, null);
});

test('the roster validates whole: duplicate aces, a death without its interval, a missing roster with aces and an alive ace with a death are refused', () => {
  assert.throws(() => validateAces(reply([alive, alive])));
  assert.throws(() => validateAces(reply([{ ...dead, death: { ...dead.death!, lastAlive: 124 } }])), 'last alive precedes dead by');
  assert.throws(() => validateAces(reply([{ ...alive, death: dead.death }])));
  assert.throws(() => validateAces({ ...reply(), missing: 'AirAces was not read' }));
  assert.deepEqual(validateAces({ ...reply([]), readHour: null, missing: 'AirAces was not read' }).aces, []);
  assert.throws(() => validateAces(reply([{ ...alive, wing: '7', aircraft: [{ id: '80', name: null, missing: null }] }])), 'an aircraft without a name says why');
  assert.throws(() => validatePortraits({ portraits: [{ id: '1', image: 3, missing: null }], warnings: [] }));
});

test('newest first sorts by first sighting over every recorded country, unrecorded last', () => {
  assert.deepEqual(DEFAULT_SORT, { column: 'date', direction: 'descending' });
  assert.deepEqual(OPTIONS.map(option => option.key), ['tableSort', 'narrowTable']);
  assert.deepEqual(definition.countries, { max: Number.MAX_SAFE_INTEGER, all: true });
  assert.ok(!('availability' in definition), 'the roster reply says whether aces were recorded');
  const rows = aceRows(reply(), 124);
  assert.equal(sortRows(rows, aceDate, 'descending', row => row.key)[0].key, '3');
  assert.equal(sortRows(rows, aceNameValue, 'ascending', row => row.key)[0].key, '2');
  const many = Array.from({ length: 100_000 }, (_, index) => ({ ...alive, id: String(index + 1), tag: `opaque country ${Math.floor(index / 20)}:Ω` }));
  const served = aceRows(validateAces(reply(many)), 124);
  assert.equal(served.length, 100_000);
  assert.equal(new Set(served.map(row => row.ace.tag)).size, 5000);
});
