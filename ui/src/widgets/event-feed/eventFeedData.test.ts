import assert from 'node:assert/strict';
import test from 'node:test';
import type { CountryGroupReply, EventFeedReply, EventFeedRow } from '../../generated/dashboard';
import { countryMatch } from '../../dashboard/groups.ts';
import { logCountryRequests } from '../../charts/log/logData.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { CATEGORIES, OPTIONS, definition } from './definition.ts';
import { eventCountryName, eventDescription, eventTooltip, filteredEventRows, notRecorded, validateEventFeedPage, validateEventFeedReply } from './eventFeedData.ts';
import { logPlaybackAt, logPlaybackIndex } from '../../charts/log/logPlayback.ts';
import type { FeedFilter } from './eventFeedData';

const all: FeedFilter = { categories: new Set(CATEGORIES.map(category => category.value)),
  country: () => true, firstHour: null };
const row = (kind: EventFeedRow['kind'], category: EventFeedRow['category'], hour = 120, more: Partial<EventFeedRow> = {}): EventFeedRow => ({
  id: JSON.stringify([kind, hour]), kind, category, hour, readHour: hour, nameHour: hour, exact: false,
  tags: ['mod | country'], item: null, amount: null, ...more,
});
const reply = (rows: EventFeedRow[] = []): EventFeedReply => ({ branchId: '1', firstHour: 100, lastHour: 200,
  windows: [{ firstHour: 100, lastHour: 200, count: Math.max(1, rows.length), categoryCounts: {
    warsPeace: rows.filter(row => row.category === 'warsPeace').length,
    diplomacy: rows.filter(row => row.category === 'diplomacy').length,
    politics: rows.filter(row => row.category === 'politics').length,
    milestones: rows.filter(row => row.category === 'milestones').length + (rows.length ? 0 : 1),
  } }], rows, items: [], modifiers: [], texts: [], unavailable: [], warnings: [] });
const named = { token: 'mod/item', name: 'A modded item', description: null,
  icon: 'data:image/png;base64,AA==', idea: null };

test('event tooltips show gameplay caveats without country-label provenance', () => {
  for (const [kind, category, note] of [
    ['warStarted', 'warsPeace', 'War entry; declaration or joining unknown.'],
    ['ceasedToExist', 'milestones', 'Lost all owned states.'],
    ['nukesIncreased', 'milestones', 'Stockpile increase.'],
  ] as const) {
    const data = reply([row(kind, category, 150, { nameHour: 149, amount: 1 })]);
    const at = filteredEventRows(data, all)[0].item;
    const tip = eventTooltip(at, () => 'Country')[0];
    assert.equal(tip.note, note);
    assert.ok(!tip.rows.some(row => row.label === 'Country last observed'));
    assert.ok(tip.rows.some(row => row.label === 'Date'));
  }
});

test('native roles stay unknown in their original positions and remain visible for all countries', () => {
  const data = reply([row('annexed', 'warsPeace', 115, {
    exact: true, readHour: 140, tags: ['mod,partner'], participants: [null, 'mod,partner'],
  }), row('warJoined', 'warsPeace', 116, {
    exact: true, readHour: 140, tags: [], participants: [null, null],
  })]);
  const rows = filteredEventRows(validateEventFeedReply(data), { ...all, allCountries: true });
  assert.equal(rows.length, 2);
  assert.equal(eventDescription(rows[1].item, tag => tag), 'A country not identified annexed mod,partner');
  assert.equal(eventDescription(rows[0].item, tag => tag), 'A country not identified joined the war against a country not identified');
  assert.equal(filteredEventRows(data, { ...all, allCountries: false, country: () => false }).length, 0);
  assert.equal(filteredEventRows(data, { ...all, allCountries: false, country: tag => tag === 'mod,partner' }).length, 1);
  assert.equal(logPlaybackAt(logPlaybackIndex(rows), 139).rows.length, 0);
  assert.equal(logPlaybackAt(logPlaybackIndex(rows), 140).rows.length, 2);
  assert.throws(() => validateEventFeedReply({ ...data, rows: [{ ...data.rows[0], participants: ['another country', null] }] }), /malformed/);
  assert.throws(() => validateEventFeedReply({ ...data, rows: [{ ...data.rows[0], participants: undefined }] }), /malformed/);
});

test('unchanged playback hours preserve feed row references while read, filter and content changes update them', () => {
  const data = reply([row('focusCompleted', 'milestones', 110, { item: 0 }),
    row('lawChanged', 'politics', 120), row('projectFinished', 'milestones', 130)]);
  data.items = [named];
  const index = logPlaybackIndex(filteredEventRows(data, all));
  const first = logPlaybackAt(index, 110);
  assert.equal(logPlaybackAt(index, 119, first), first);
  const added = logPlaybackAt(index, 120, first);
  assert.notEqual(added, first);
  assert.deepEqual(added.rows.map(row => row.item.event.kind), ['lawChanged', 'focusCompleted']);
  const filtered = logPlaybackAt(logPlaybackIndex(filteredEventRows(data, { ...all, categories: new Set(['politics']) })), 120, added);
  assert.deepEqual(filtered.rows.map(row => row.item.event.kind), ['lawChanged']);
  const content = { ...data, items: [{ ...named, name: 'A changed content name' }] };
  const changed = logPlaybackAt(logPlaybackIndex(filteredEventRows(content, all)), 110, first);
  assert.equal(changed.rows[0].item.item!.name, 'A changed content name');
});

test('country labels distinguish pending, failed and completed missing name reads', () => {
  assert.equal(eventCountryName('opaque | tag', { name: 'opaque | tag' }), 'opaque | tag (loading name…)');
  assert.equal(eventCountryName('opaque | tag', { name: 'opaque | tag', known: false }), 'opaque | tag (name not known)');
  assert.equal(eventCountryName('opaque | tag', { name: 'opaque | tag', known: false, nameFailed: true }), 'opaque | tag (name not recorded)');
  assert.equal(eventCountryName('opaque | tag', { name: 'Recorded name', known: true }), 'Recorded name');
});

test('every recorded event has honest wording and keeps countries as opaque identities', () => {
  const cases: [EventFeedRow['kind'], EventFeedRow['category'], string][] = [
    ['warStarted', 'warsPeace', 'became at war with'], ['peace', 'warsPeace', 'was no longer at war with'],
    ['capitulated', 'warsPeace', 'capitulated'], ['ceasedToExist', 'warsPeace', 'ceased to exist'],
    ['factionJoined', 'diplomacy', 'joined'], ['factionLeft', 'diplomacy', 'left'],
    ['becameSubject', 'diplomacy', 'became a subject of'], ['guaranteeGranted', 'diplomacy', 'guaranteed the independence of'],
    ['rulingPartyChanged', 'politics', 'changed its ruling party to'], ['lawChanged', 'politics', 'adopted'],
    ['advisorHired', 'politics', 'appointed'], ['focusCompleted', 'milestones', 'completed focus'],
    ['projectFinished', 'milestones', 'completed special project'],
    ['nukesIncreased', 'milestones', 'increased its nuclear stockpile by 2 bombs'],
  ];
  const data = reply(cases.map(([kind, category], index) => row(kind, category, 120 + index,
    { tags: ['mod | country', 'mod,partner'], item: 0, amount: kind === 'nukesIncreased' ? 2 : null })));
  data.items = [named];
  const rows = filteredEventRows(validateEventFeedReply(data), all);
  assert.equal(rows.length, cases.length);
  for (const [kind, , words] of cases) {
    const at = rows.find(candidate => candidate.item.event.kind === kind)!;
    const text = eventDescription(at.item, tag => tag === 'mod | country' ? 'Country at this hour' : 'Partner at this hour');
    assert.ok(text.includes(words), text);
    assert.ok(text.startsWith('Country at this hour'), text);
    assert.equal(at.by, false, 'a listed event shows its day, never "by"');
  }
});

test('native dates stay exact but wait for their supplying read, and every row shows only its day', () => {
  const data = reply([row('focusCompleted', 'milestones', 115, { exact: true, readHour: 140, nameHour: 115 }),
    row('lawChanged', 'politics', 120)]);
  const index = logPlaybackIndex(filteredEventRows(data, all));
  assert.deepEqual(logPlaybackAt(index, 119).rows, []);
  assert.equal(logPlaybackAt(index, 120).rows[0].by, false);
  const exact = logPlaybackAt(index, 140).rows.find(candidate => candidate.item.event.kind === 'focusCompleted')!;
  assert.equal(exact.hour, 115);
  assert.equal(exact.revealedHour, 140);
  assert.equal(exact.by, false);
  const labels = eventTooltip(exact.item, tag => tag)[0].rows?.map(row => row.label) ?? [];
  assert.equal(labels[0], 'Date');
  assert.ok(!labels.some(label => /observed by|supplying|Native date/i.test(String(label))), 'the hover gives the day, not reads or bounds');
  assert.ok(!labels.includes('Artwork'), 'an event without a picture says nothing about artwork');
  assert.doesNotMatch(String(eventTooltip(exact.item, tag => tag)[0].rows?.[0].value), /·/, 'no hour');
  assert.deepEqual(filteredEventRows(data, { ...all, firstHour: 121 }), []);
});

test('modded focus, law and project retain content names and artwork, with the shared idea tooltip', () => {
  const data = reply([row('focusCompleted', 'milestones', 110, { item: 0 }), row('lawChanged', 'politics', 120, { item: 1 }),
    row('projectFinished', 'milestones', 130, { item: 2 })]);
  data.items = ['Focus from a mod', 'Law from a mod', 'Project from a mod'].map(name => ({ ...named, name }));
  data.items[1].idea = { token: 'mod/item', category: 'mod_category', slot: 'mod_slot', law: true, designer: false,
    effectsKnown: true, unknown: null, effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [] };
  const rows = filteredEventRows(validateEventFeedReply(data), all);
  for (const item of rows) {
    assert.equal(item.item.item?.icon, named.icon);
    assert.ok(eventDescription(item.item, tag => tag).includes(item.item.item!.name!));
  }
  assert.equal(filteredEventRows(data, { ...all, categories: new Set(['politics']) }).length, 1);
  const law = rows.find(item => item.item.event.kind === 'lawChanged')!;
  assert.ok(eventTooltip(law.item, tag => tag)[0].title?.includes('Law from a mod'));
});

test('renames and disappeared countries request only their row-hour identities without later names', () => {
  const data = reply([row('focusCompleted', 'milestones', 110), row('rulingPartyChanged', 'politics', 150),
    row('ceasedToExist', 'warsPeace', 170, { nameHour: 169 })]);
  const rows = filteredEventRows(data, all);
  assert.deepEqual(logCountryRequests(rows, candidate => ({ hour: candidate.item.event.nameHour, tags: candidate.item.event.tags })),
    [{ hour: 169, tag: 'mod | country' }, { hour: 150, tag: 'mod | country' }, { hour: 110, tag: 'mod | country' }]);
  const names = new Map([[110, 'Old name'], [150, 'New name'], [169, 'Last observed name']]);
  assert.ok(eventDescription(rows[2].item, () => names.get(rows[2].item.event.nameHour)!).startsWith('Old name'));
  assert.ok(eventDescription(rows[0].item, () => names.get(169)!).startsWith('Last observed name'));
  assert.match(eventTooltip(rows[0].item, tag => tag)[0].note!, /Lost all owned states/);
});

test('country filters use row hours and last-known membership for left and vanished countries', () => {
  const data = reply([row('peace', 'warsPeace', 110, { tags: ['one', 'two'] }), row('peace', 'warsPeace', 150, { tags: ['one', 'two'] })]);
  const filter = { ...all, country: (tag: string, hour: number) => tag === 'two' && hour === 110 };
  assert.deepEqual(filteredEventRows(data, filter).map(item => item.hour), [110]);
  const match = countryMatch({ kind: 'all' }, [], undefined, 200);
  assert.equal(match.matches('AI or human | opaque', 110), true);
  assert.equal(filteredEventRows(data, { ...all, country: () => false }).length, 0);
  const groups: CountryGroupReply = { branchId: '1', warnings: [], identityChanges: [], humans: [], wars: [],
    countryIdentities: [{ tag: 'two', identities: [{ firstHour: 100, lastHour: 200, slot: 1, countryId: 10 }] }],
    warStarts: [], capitulations: [], unobserved: { humans: [], wars: [], factions: [{ firstHour: 145, lastHour: 155 }] },
    factions: [{ id: '7', name: 'Modded faction', nameReason: null, recordedName: 'mod_faction', firstHour: 100, lastHour: 200,
      names: [{ firstHour: 100, lastHour: 144, name: 'Modded faction', reason: null },
        { firstHour: 156, lastHour: 200, name: 'Modded faction', reason: null }],
      members: [{ tag: 'two', firstHour: 100, lastHour: 200 }], leaders: [], incomplete: [] }] };
  const selected = countryMatch({ kind: 'group', group: { type: 'faction', id: '7' }, membership: 'hour', aggregate: false }, [], groups, 200);
  assert.deepEqual(filteredEventRows(data, { ...all, country: selected.matches }).map(candidate => candidate.hour), [110]);
  const ended = reply([row('factionLeft', 'diplomacy', 145, { tags: ['two'], nameHour: 144 }),
    row('ceasedToExist', 'warsPeace', 150, { tags: ['two'], nameHour: 144 })]);
  assert.deepEqual(filteredEventRows(ended, { ...all, country: selected.matches }).map(candidate => candidate.hour), [150, 145]);
});

test('missing captures have one explicit adapter, and bad icons do not discard named mod content', () => {
  const data = reply([row('focusCompleted', 'milestones', 120, { item: 0 })]);
  data.items = [{ ...named, icon: 'javascript:bad' }];
  data.unavailable = [{ kind: 'drops', label: 'Nuclear drops', reason: 'Only stockpile amounts were captured.', count: null },
    { kind: 'mod/refused', label: 'Modded form', reason: 'Unsupported scripted artwork: mod/refused.', count: 3 }];
  const checked = validateEventFeedReply(data);
  assert.equal(checked.items[0].icon, null);
  assert.equal(checked.items[0].name, named.name);
  assert.match(notRecorded(checked)[0], /Not recorded.*Only stockpile/);
  assert.ok(notRecorded(checked)[0].includes('Not recorded — '));
  assert.match(notRecorded(checked)[1], /mod\/refused.*3 counted/);
  assert.throws(() => validateEventFeedReply({ ...data, rows: [{ ...data.rows[0], readHour: 119 }] }));
  assert.throws(() => validateEventFeedReply({ ...data, rows: [{ ...data.rows[0], item: 99 }] }));
  assert.throws(() => validateEventFeedReply({ ...data, rows: [data.rows[0], data.rows[0]] }));
});

test('country rows keep every observed match and do not invent empty-capture events', () => {
  const data = reply(Array.from({ length: 8 }, (_, index) => row('nukesIncreased', 'milestones', 120,
    { id: `nuke/${index}`, tags: [`mod country ${index}`], amount: 1 })));
  assert.equal(filteredEventRows(validateEventFeedReply(data), all).length, 8);
  assert.equal(filteredEventRows(reply([]), all).length, 0);
});

test('country filtering happens once per prepared page and never on unchanged playback hours', () => {
  const data = reply(Array.from({ length: 8 }, (_, index) => row('focusCompleted', 'milestones', 120,
    { id: `focus/${index}`, tags: [`opaque ${index}`], readHour: 140 })));
  let countryChecks = 0;
  const index = logPlaybackIndex(filteredEventRows(data, { ...all, country: () => { countryChecks++; return true; } }));
  assert.equal(countryChecks, 8);
  const before = logPlaybackAt(index, 120);
  for (let hour = 121; hour < 140; hour++) assert.equal(logPlaybackAt(index, hour, before), before);
  const arrived = logPlaybackAt(index, 140, before);
  assert.equal(arrived.rows.length, 8);
  assert.equal(logPlaybackAt(index, 200, arrived), arrived);
  assert.equal(countryChecks, 8);
});

test('war and peace sentences list every counterparty and guarantees retain their recorded direction', () => {
  const data = reply([row('warStarted', 'warsPeace', 110, { tags: ['actor', 'one', 'two', 'three'] }),
    row('peace', 'warsPeace', 120, { tags: ['actor', 'one', 'two'] }),
    row('guaranteeGranted', 'diplomacy', 130, { tags: ['guarantor', 'guaranteed'] })]);
  const rows = filteredEventRows(data, all);
  assert.equal(eventDescription(rows[2].item, tag => tag), 'actor became at war with one, two and three');
  assert.equal(eventDescription(rows[1].item, tag => tag), 'actor was no longer at war with one and two');
  assert.equal(eventDescription(rows[0].item, tag => tag), 'guarantor guaranteed the independence of guaranteed');
});

test('defaults match the approved log and obsolete technology options cannot enable events', () => {
  const options = resolveOptions(OPTIONS, {});
  assert.equal(options.density, 'comfortable');
  assert.equal(options.months, false);
  assert.equal(options.chips, true);
  assert.equal(Object.hasOwn(options, 'technologies'), false);
  assert.deepEqual(resolveOptions(OPTIONS, { technologies: true }), options);
  assert.deepEqual(resolveOptions(OPTIONS, { technologies: false }), options);
  assert.equal(options.artwork, true);
  assert.equal(definition.countries.all, true);
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  for (const category of CATEGORIES) assert.equal(options[category.value], true);
});

test('technology completion rows are outside the Event feed contract', () => {
  const data = reply([row('focusCompleted', 'milestones')]);
  assert.throws(() => validateEventFeedReply({ ...data, rows: [{ ...data.rows[0], kind: 'technologyResearched' }] }),
    /event feed reply is malformed/);
});

test('native causes say how an event happened, name the ally and the state, and older rows say none was recorded', () => {
  const cause = { how: 'a call to arms', within: 'a diplomatic action', conference: null, state: null,
    ally: 'mod | ally', reason: 'ally', notes: ['against every enemy of the ally side'] };
  const data = validateEventFeedReply(reply([
    row('warJoined', 'warsPeace', 120, { exact: true, tags: ['mod | joiner', 'mod | enemy'],
      participants: ['mod | joiner', 'mod | enemy'], cause }),
    row('stateTransferred', 'warsPeace', 121, { exact: true, tags: ['mod | taker', 'mod | loser'],
      participants: ['mod | taker', 'mod | loser'], cause: { ...cause, how: 'an annexation', ally: null, reason: null,
        notes: [], state: 64, conference: '30064771121' } }),
    row('warEnded', 'warsPeace', 122, { exact: true, tags: ['mod | a', 'mod | b'], participants: ['mod | a', 'mod | b'] }),
  ]));
  const rows = filteredEventRows(data, all);
  const state = (_hour: number, id: number) => `Danzig (${id})`;
  assert.equal(eventDescription(rows[2].item, tag => tag, state),
    'mod | joiner joined the war against mod | enemy through a call to arms, alongside mod | ally');
  assert.equal(eventDescription(rows[1].item, tag => tag, state),
    'mod | taker took Danzig (64) from mod | loser through an annexation');
  assert.equal(eventDescription(rows[0].item, tag => tag), 'mod | a and mod | b were no longer at war');
  const joined = eventTooltip(rows[2].item, tag => tag)[0].rows!;
  assert.deepEqual(joined.slice(1, 5).map(item => [item.label, item.value]), [['How', 'a call to arms'],
    ['During', 'a diplomatic action'], ['Hostility reason', 'ally'], ['against every enemy of the ally side', undefined]]);
  assert.equal(eventTooltip(rows[1].item, tag => tag)[0].rows!.find(item => item.label === 'Peace conference')?.value, '30064771121');
  assert.equal(eventTooltip(rows[0].item, tag => tag)[0].rows!.find(item => item.label === 'Cause')?.value,
    'Not recorded in this recording');
  assert.throws(() => validateEventFeedReply({ ...data, rows: [{ ...data.rows[0], cause: { ...cause, ally: 7 } }] }), /malformed/);
  assert.throws(() => validateEventFeedReply(reply([row('lawChanged', 'politics', 120, { cause })])), /malformed/);
});

test('a nuclear strike names its bomb, launcher and target owner, with totals only for fully recorded parts', () => {
  const strike = { province: 13000, state: 2, buildingLevels: 2.5, warSupport: -20, units: 3, manpowerLost: 2500,
    equipmentLost: null, notRecorded: ['equipment: equipment destruction writers not forwarded (1×)'] };
  const data = reply([row('nuclearStrike', 'warsPeace', 115, {
    exact: true, readHour: 116, tags: ['mod | country', 'mod,partner'], participants: ['mod | country', 'mod,partner'],
    item: 0, strike,
  }), row('nuclearStrike', 'warsPeace', 117, {
    exact: true, readHour: 117, tags: ['mod | country'], participants: ['mod | country', null],
    strike: { ...strike, state: null, notRecorded: [] },
  })]);
  data.items = [{ ...named, name: 'Thermonuclear Bomb' }];
  const rows = filteredEventRows(validateEventFeedReply(data), all);
  const at = (hour: number) => rows.find(candidate => candidate.item.event.hour === hour)!.item;
  assert.equal(eventDescription(at(115), tag => tag), 'mod | country dropped Thermonuclear Bomb on mod,partner');
  assert.equal(eventDescription(at(117), tag => tag),
    'mod | country dropped a nuclear bomb of a type not known on a province without a state');
  const tooltip = eventTooltip(at(115), tag => tag, (hour, state) => hour === 115 && state === 2 ? 'Moscow' : null)[0].rows!;
  const value = (label: string) => tooltip.find(entry => entry.label === label)?.value;
  assert.equal(value('State'), 'Moscow');
  assert.equal(value('Building levels destroyed'), (2.5).toLocaleString());
  assert.equal(value('War support'), '-20%');
  assert.equal(value('Equipment destroyed'), 'Not fully recorded');
  assert.equal(value('Not recorded'), strike.notRecorded[0]);
  assert.throws(() => validateEventFeedReply({ ...data, rows: [{ ...data.rows[0], strike: undefined }] }), /malformed/);
  assert.throws(() => validateEventFeedReply(reply([row('lawChanged', 'politics', 120, { strike })])), /malformed/);
});

test('kinds without game artwork have their own icon', async () => {
  const { EVENT_ICONS } = await import('./definition.ts');
  assert.deepEqual([EVENT_ICONS.warDeclared, EVENT_ICONS.civilWarWarTransferred, EVENT_ICONS.peace, EVENT_ICONS.capitulated,
    EVENT_ICONS.becameSubject, EVENT_ICONS.rulingPartyChanged], ['swords', 'flag-torn', 'olive-branch', 'flag', 'link', 'ballot']);
  assert.deepEqual([EVENT_ICONS.warEnded, EVENT_ICONS.peaceConferenceStarted, EVENT_ICONS.nuclearStrike], ['olive-branch', 'handshake', 'sunburst']);
});

test('prepared cursor pages retain duplicate native occurrences and keep IDs inside the row owner', () => {
  const first = row('warDeclared', 'warsPeace', 125, { id: 'native-99', exact: true, readHour: 150, tags: ['actor', 'target'], participants: ['actor', 'target'] });
  const data = { ...reply([first, { ...first, readHour: 151 }]), windows: [] };
  const page = validateEventFeedPage({ reply: data, nextCursor: 'keeps-live-200-native-125' });
  assert.equal(page.reply.rows.length, 2);
  const rows = filteredEventRows(page.reply, all);
  assert.notEqual(rows[0].key, rows[1].key);
  assert.deepEqual(rows.map(row => row.item.event.id), ['native-99', 'native-99']);
  assert.throws(() => validateEventFeedPage({ reply: data, nextCursor: 125 }), /malformed/);
  assert.equal(validateEventFeedPage({ reply: { ...data, rows: [] }, nextCursor: 'next' }).nextCursor, 'next',
    'an obsolete raw project page retains its advancing continuation');
  assert.throws(() => validateEventFeedPage({ reply: data, nextCursor: '' }), /no progress/);
  assert.equal(validateEventFeedPage({ reply: { ...data, rows: [] }, nextCursor: null }).reply.rows.length, 0);
  assert.throws(() => validateEventFeedPage({ reply: reply(), nextCursor: null }), /manifest/);
});
