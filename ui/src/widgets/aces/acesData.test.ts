import test from 'node:test';
import assert from 'node:assert/strict';
import { aceRows, aceName, aceNameValue, aceStatus, aceJump, aceTooltip, aircraftName, wingName, validateAces, validateAcesWindow, validatePortraits } from './acesData.ts';
import { DEFAULT_SORT, OPTIONS, definition } from './definition.ts';
import { sortRows } from '../../charts/table/logic.ts';
import { availabilityRequest, availabilityReason, validateWidgetAvailability } from '../../dashboard/widgetAvailability.ts';
import { createWidget } from '../../dashboard/model.ts';
import { countryMatch } from '../../dashboard/groups.ts';
import type { BoundTrack, CountrySelection } from '../../dashboard/model';
import type { AirAcesManifestReply, AirAcesWindowReply, AirAceRun, CountryGroupReply } from '../../generated/dashboard';
import '../definitions.ts';

const alive: AirAceRun = { firstHour: 100, lastHour: 123, tag: 'opaque tag:Ω', name: 0, callsign: 1, wing: null, wingName: null, aircraft: [], alive: true, death: null };
const dead: AirAceRun = { ...alive, firstHour: 124, lastHour: 147, alive: false, death: { lastAlive: 100, byHour: 124, killerAce: '3', killerTag: 'other tag' } };
function window(): AirAcesWindowReply {
  return { branchId: '1', window: 0, firstHour: 100, lastHour: 147, warnings: [], texts: ['Zed', 'Storm', 'Alpha', 'Killer then', 'Killer later'], aces: [
    { id: '1', firstSeen: 100, runs: [alive, dead] },
    { id: '2', firstSeen: 124, runs: [{ ...dead, name: 2, death: { lastAlive: null, byHour: null, killerAce: null, killerTag: null } }] },
    { id: '3', firstSeen: 100, runs: [{ ...alive, name: 3, lastHour: 130 }, { ...alive, name: 4, firstHour: 131, lastHour: 147 }] },
  ] };
}
function reply(): AirAcesManifestReply {
  return { branchId: '1', firstHour: 100, lastHour: 147, available: true, unavailable: null, observed: [{ firstHour: 100, lastHour: 147 }], readHours: [100, 124, 131, 140], warnings: [],
    windows: [{ firstHour: 100, lastHour: 147 }], aces: window().aces.map(({ id, firstSeen }) => ({ id, firstSeen })), variants: { '80': [
      { firstHour: 100, name: 'First design', known: true, design: 'first' },
      { firstHour: 131, name: 'Later design', known: true, design: 'later' },
      { firstHour: 140, name: null, known: false, design: null },
    ] } };
}
test('Aces defaults to all countries and filters chosen owners and smart groups at each supplying read', () => {
  assert.deepEqual(definition.countries, { max: Number.MAX_SAFE_INTEGER, all: true });
  assert.deepEqual(createWidget('aces', 'main', 'aces-test').countries, { kind: 'all' });
  const data = reply();
  const detail = window();
  detail.aces[1].runs[0] = { ...detail.aces[1].runs[0], tag: 'other mod country' };
  detail.aces[2].runs = detail.aces[2].runs.map(run => ({ ...run, tag: null }));
  const owner = alive.tag!;
  const groups: CountryGroupReply = {
    branchId: '1', warnings: [], humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [owner, 'other mod country'].map((tag, slot) => ({ tag,
      identities: [{ firstHour: 100, lastHour: 147, slot, countryId: slot + 1 }] })),
    unobserved: { humans: [], factions: [], wars: [] },
  };
  groups.humans = [
    { tag: owner, firstHour: 100, lastHour: 100 },
    { tag: 'other mod country', firstHour: 101, lastHour: 147 },
  ];
  const keys = (selection: CountrySelection, hour = 124, recorded: CountryGroupReply | null = groups, fallback: string[] = [owner]) => {
    const match = countryMatch(selection, fallback, recorded ?? undefined, data.lastHour);
    return match.ready ? aceRows(data, detail, hour,
      (tag, read) => tag === null ? selection.kind === 'all' : match.matches(tag, read)).map(row => row.key) : [];
  };
  assert.deepEqual(keys({ kind: 'all' }), ['1', '2', '3'], 'unknown owners remain visible for all countries');
  assert.deepEqual(keys({ kind: 'selected' }), ['1']);
  assert.deepEqual(keys({ kind: 'tags', tags: ['other mod country'] }), ['2']);
  const chosen = aceRows(data, detail, 124, tag => tag === owner)[0];
  assert.equal(aceTooltip(chosen, String).rows.find(row => row.label === 'Killer ace')?.value, 'Killer then', 'filtering keeps the full killer-name roster');
  assert.deepEqual(keys({ kind: 'selected' }, 124, groups, []), [], 'a missing default country does not broaden the selection');
  const group = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
  assert.deepEqual(keys(group, 123), ['1'], 'carried rows match read 100, not playhead 123');
  assert.deepEqual(keys(group, 124), ['2']);
  assert.deepEqual(keys({ ...group, membership: 'end' }, 123), []);
  assert.deepEqual(keys({ ...group, membership: 'ever' }), ['1', '2']);
  assert.deepEqual(keys(group, 124, null), [], 'pending membership does not show all countries');
  assert.deepEqual(keys(group, 124, { ...groups, unobserved: { ...groups.unobserved, humans: [{ firstHour: 124, lastHour: 124 }] } }), []);
});
test('picker requests the cheap catalogue and accepts the actual availability contract', () => {
  const track = { source: { id: 'source', revision: '2' }, branch: { id: '1' } } as BoundTrack;
  assert.deepEqual(availabilityRequest(definition, track), { sourceId: 'source', revision: '2', product: 'air-aces', params: { view: 'catalogue', branchId: '1' } });
  const catalogue = { ...reply(), aces: [], windows: [], observed: [], readHours: [], variants: {} };
  assert.equal(availabilityReason(true, true, validateWidgetAvailability(validateAces(catalogue))), null);
  const absent = { ...catalogue, available: false, unavailable: 'AirAces not recorded' };
  assert.equal(availabilityReason(true, true, validateWidgetAvailability(validateAces(absent))), 'AirAces not recorded');
  assert.throws(() => validateAces({ ...catalogue, unavailable: undefined }));
});
test('first sightings and deaths are bounds; killer names come from their own hour-specific entry', () => {
  const data = validateAces(reply());
  const detail = validateAcesWindow(window());
  assert.equal(aceRows(data, detail, 99).length, 0);
  assert.equal(aceRows(data, detail, 123).length, 2);
  assert.match(aceStatus(aceRows(data, detail, 123)[0]), /^Ace by /);
  const rows = aceRows(data, detail, 124);
  assert.equal(rows.length, 3);
  assert.equal(rows[0].run.alive, false);
  assert.match(aceStatus(rows[0]), /^Killed by /);
  assert.equal(aceStatus(rows[1]), 'Killed, date not recorded');
  assert.equal(aceTooltip(rows[1], String).rows.find(r => r.label === 'Killer country')?.value, 'Not recorded');
  assert.equal(aceJump(rows[0]), 124);
  assert.equal(aceJump(rows[1]), 124);
  const tip = aceTooltip(rows[0], tag => `At read: ${tag}`);
  assert.equal(tip.note, undefined, 'bounds carry uncertainty without recorder notes');
  assert.equal(tip.title, 'Zed “Storm”');
  assert.equal(tip.rows[0].label, 'At read: opaque tag:Ω');
  assert.equal(tip.rows.find(r => r.label === 'Killer ace')?.value, 'Killer then');
  assert.equal(aceTooltip(aceRows(data, detail, 131)[0], String).rows.find(r => r.label === 'Killer ace')?.value, 'Killer later');
  assert.equal(detail.aces[0].runs.length, 2, 'killer name changes need no victim run');
  assert.match(tip.rows.find(r => r.label === 'Death interval')?.value ?? '', /After .*; by /);
  assert.ok(!tip.rows.some(r => r.label === 'Type' || r.label === 'Aircraft'));
  data.readHours = [100, 101, 124, 131, 140];
  assert.equal(aceRows(data, detail, 123)[0].hour, 101, 'equal facts still date labels to the last actual read');
});

test('unknown ace and killer names do not expose their IDs', () => {
  const detail = window();
  detail.aces = detail.aces.map(ace => ({ ...ace, runs: ace.runs.map(run => ({ ...run, name: null })) }));
  const row = aceRows(reply(), detail, 124)[0];
  assert.equal(aceName(row), 'Unknown ace');
  assert.equal(aceTooltip(row, String).rows.find(item => item.label === 'Killer ace')?.value, 'Unknown ace');
});
test('one aircraft-id run uses shared variant names; gaps and missing assignments stay explicit', () => {
  const data = reply();
  const detail = window();
  detail.aces = [{ id: '1', firstSeen: 100, runs: [{ ...alive, lastHour: 147, wing: '7', aircraft: ['80'] }] }];
  assert.equal(aircraftName(aceRows(data, detail, 124)[0]), 'First design');
  assert.equal(aircraftName(aceRows(data, detail, 131)[0]), 'Later design');
  assert.equal(aircraftName(aceRows(data, detail, 140)[0]), 'Unknown aircraft');
  assert.equal(detail.aces[0].runs.length, 1);
  data.observed = [{ firstHour: 100, lastHour: 110 }, { firstHour: 124, lastHour: 147 }];
  const run = detail.aces[0].runs[0];
  detail.aces[0].runs = [{ ...run, lastHour: 110 }, { ...run, firstHour: 124 }];
  assert.equal(aceRows(data, detail, 111).length, 0);
  const row = aceRows(data, detail, 100)[0];
  assert.equal(wingName(row), 'Wing name not recorded');
  row.run = { ...row.run, aircraft: null };
  assert.equal(aircraftName(row), 'Aircraft not recorded');
  row.run = { ...row.run, wing: null };
  assert.equal(wingName(row), 'No wing');
  assert.equal(aircraftName(row), '');
});
test('roster lookup is reused across reads of one reply and isolated between replies', () => {
  const data = reply();
  const detail = window();
  const first = aceRows(data, detail, 100)[0].roster;
  assert.equal(aceRows(data, detail, 131)[0].roster, first);
  assert.notEqual(aceRows(reply(), window(), 131)[0].roster, first);
});
test('the approved kit sort choices and every recorded country remain available', () => {
  assert.deepEqual(DEFAULT_SORT, { column: 'date', direction: 'descending' });
  assert.equal(OPTIONS.length, 1);
  const sort = OPTIONS[0];
  assert.ok(sort.kind === 'choice');
  assert.deepEqual(sort.choices.map(choice => choice.label), ['Newest first', 'Name']);
  const rows = aceRows(reply(), window(), 124);
  assert.equal(sortRows(rows, row => row.ace.firstSeen, 'descending', row => row.key)[0].key, '2');
  assert.equal(sortRows(rows, aceNameValue, 'ascending', row => row.key)[0].key, '2');
  const many = reply();
  const detail = window();
  detail.aces = Array.from({ length: 100_000 }, (_, index) => ({ id: String(index + 1), firstSeen: 100, runs: [{ ...alive, tag: `opaque country ${Math.floor(index / 20)}:Ω` }] }));
  many.aces = detail.aces.map(({ id, firstSeen }) => ({ id, firstSeen }));
  const served = aceRows(validateAces(many), validateAcesWindow(detail), 100);
  assert.equal(served.length, 100_000);
  assert.equal(new Set(served.map(row => row.run.tag)).size, 5000);
});
test('validators refuse malformed dates, text references, windows, variant chronology, duplicate ids and incomplete portraits', () => {
  assert.throws(() => validateAces({ ...reply(), aces: [reply().aces[0], reply().aces[0]] }));
  assert.throws(() => validateAcesWindow({ ...window(), aces: [window().aces[0], window().aces[0]] }));
  assert.throws(() => validateAcesWindow({ ...window(), aces: [{ id: '1', firstSeen: 100, runs: [{ ...dead, death: { ...dead.death, byHour: 148 } }] }] }));
  assert.throws(() => validateAcesWindow({ ...window(), texts: [] }));
  assert.throws(() => validateAcesWindow({ ...window(), aces: [{ id: '1', firstSeen: 100, runs: [{ ...alive, name: -1 }] }] }));
  assert.throws(() => validateAces({ ...reply(), windows: [{ firstHour: 100, lastHour: 148 }] }));
  assert.throws(() => validateAces({ ...reply(), variants: { '80': [...reply().variants['80']!].reverse() } }));
  assert.throws(() => validateAces({ ...reply(), variants: { 'not an id': [] } }));
  assert.throws(() => validatePortraits({ portraits: [{ id: '1', image: 3, missing: null }], warnings: [] }));
});
test('a block carries its earlier supplying read and original name, then changes text at the new read', () => {
  const data = reply();
  data.windows = [{ firstHour: 100, lastHour: 123 }, { firstHour: 124, lastHour: 147 }];
  data.readHours = [100, 131];
  const detail = validateAcesWindow({ ...window(), window: 1, firstHour: 124, aces: [window().aces[2]] });
  const carried = aceRows(data, detail, 124)[0];
  assert.equal(carried.hour, 100);
  assert.equal(aceNameValue(carried), 'Killer then');
  assert.equal(aceNameValue(aceRows(data, detail, 131)[0]), 'Killer later');
  assert.equal(aceRows(data, detail, 123).length, 0);
  const gap = { ...data, observed: [{ firstHour: 100, lastHour: 123 }, { firstHour: 131, lastHour: 147 }] };
  assert.equal(aceRows(gap, detail, 124).length, 0, 'a loaded carried run does not fill an observation gap');
});

test('text slot zero is a present callsign, and null stays missing', () => {
  const detail = window();
  detail.aces = [{ id: '1', firstSeen: 100, runs: [{ ...alive, callsign: 0 }] }];
  assert.equal(aceTooltip(aceRows(reply(), detail, 100)[0], String).title, 'Zed \u201cZed\u201d');
  detail.aces[0].runs[0] = { ...alive, callsign: null };
  assert.equal(aceTooltip(aceRows(reply(), detail, 100)[0], String).title, 'Zed');
});

test('assigned wing text slot zero preserves an empty name while null stays unrecorded', () => {
  const detail = validateAcesWindow({ ...window(), texts: [''], aces: [
    { id: '1', firstSeen: 100, runs: [{ ...alive, name: null, callsign: null, wing: '7', wingName: 0 }] },
  ] });
  const row = aceRows(reply(), detail, 100)[0];
  assert.equal(wingName(row), '');
  assert.equal(aceTooltip(row, String).rows.find(item => item.label === 'Air wing')?.value, '');
  row.run = { ...row.run, wingName: null };
  assert.equal(wingName(row), 'Wing name not recorded');
  assert.equal(aceTooltip(row, String).rows.find(item => item.label === 'Air wing')?.value, 'Wing name not recorded');
  row.run = { ...row.run, wing: null };
  assert.equal(wingName(row), 'No wing');
});
