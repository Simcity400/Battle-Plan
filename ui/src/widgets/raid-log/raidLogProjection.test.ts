import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { DisplayedHour, EntityKind, IdentityProvider, IdentityResult } from '../../families/plan.ts';
import { buildRowSourceFromHead, validateGenericTailReply } from '../../families/table/rowSource.ts';
import type { RaidLogRow, RaidLogTailReply, RaidTypeChip } from '../../generated/dashboard';
import { raidLogProjection, raidTypeFilterItems } from './raidLogProjection.ts';

function sampleRow(overrides: Partial<RaidLogRow> = {}): RaidLogRow {
  const hour = overrides.hour ?? 100;
  return {
    key: '1',
    hour,
    exact: true,
    observed: hour,
    ordinal: '0',
    live: true,
    status: 'preparing',
    phaseName: 'Preparing',
    outcomeMissing: null,
    raider: { tag: 'ENG', slot: 1, missing: null },
    victim: { hour, tag: 'GER', missing: null, source: 'fixedTarget' },
    raidType: 'facility_strike',
    raidTypeName: 'Facility Strike',
    typeMissing: null,
    icon: null,
    iconMissing: null,
    place: { province: 6521, provinceHour: hour, state: 64, stateHour: hour },
    placeMissing: null,
    preparation: { since: hour, hours: 96, perHour: 1, through: null },
    launchedHour: null,
    end: null,
    disappearedHour: null,
    lastSeen: null,
    ...overrides,
  };
}

function sampleReply(rows: RaidLogRow[], overrides: Partial<RaidLogTailReply> = {}): RaidLogTailReply {
  const liveCount = rows.filter(r => r.live).length;
  return {
    branchId: '1',
    total: String(rows.length),
    offset: '0',
    live: String(liveCount),
    counts: [{ key: 'active', total: String(liveCount) }],
    raidTypes: [],
    rows,
    missing: [],
    nextChange: null,
    unavailable: null,
    warnings: [],
    ...overrides,
  };
}

function makeIdentity(resolve: (kind: EntityKind, key: string, hour: number) => string | null): IdentityProvider {
  return {
    resolve(kind: EntityKind, key: string, atHour: DisplayedHour): IdentityResult {
      const name = resolve(kind, key, atHour);
      return {
        kind,
        key,
        atHour,
        holder: { status: 'absent', reason: 'not set' },
        name: name !== null ? { status: 'ready', value: name } : { status: 'absent', reason: `${kind} ${key} not recorded` },
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    },
  };
}

test('raid type chips exclude types first spelled after the displayed hour and disambiguate duplicate names', () => {
  const chips: RaidTypeChip[] = [
    { spelling: 'facility_strike', firstHour: 8, name: 'Facility Strike', icon: null, iconMissing: null },
    { spelling: 'mod_facility_strike', firstHour: 10, name: 'Facility Strike', icon: null, iconMissing: null },
    { spelling: 'sky_raid', firstHour: 12, name: 'Sky Raid', icon: null, iconMissing: null },
  ];
  const at10 = raidTypeFilterItems(chips, [], 10);
  assert.deepEqual(
    at10.map(item => [item.value, item.label, item.checked]),
    [
      ['facility_strike', 'Facility Strike (1)', true],
      ['mod_facility_strike', 'Facility Strike (2)', true],
    ],
    'a raid type first spelled at hour 12 is not in the chips at hour 10',
  );
  const at12 = raidTypeFilterItems(chips, ['sky_raid'], 12);
  assert.deepEqual(
    at12.map(item => [item.value, item.label, item.checked]),
    [
      ['facility_strike', 'Facility Strike (1)', false],
      ['mod_facility_strike', 'Facility Strike (2)', false],
      ['sky_raid', 'Sky Raid', true],
    ],
  );
});

test('a later place or country rename never reaches an earlier raid row', () => {
  const row = sampleRow({
    key: 'r-rename',
    hour: 10,
    raider: { tag: 'GER', slot: 1, missing: null },
    victim: { hour: 11, tag: 'POL', missing: null, source: 'fixedTarget' },
    place: { province: 6521, provinceHour: 10, state: 64, stateHour: 10 },
  });
  const reply = sampleReply([row]);
  const source = buildRowSourceFromHead('test:raids:rename', reply, undefined, 50)!;
  const readView = createBoundedReadView({
    hour: 50,
    rows: new Map([['main', source]]),
    identity: makeIdentity((kind, key, hour) => {
      if (kind === 'state' && key === '64') return hour < 30 ? 'Brandenburg' : 'Renamed Brandenburg';
      if (kind === 'country' && key === 'GER') return hour < 30 ? 'German Reich' : 'West Germany';
      if (kind === 'country' && key === 'POL') return hour < 30 ? 'Poland' : 'Renamed Poland';
      return key;
    }),
  });
  const model = raidLogProjection.project(readView, { artwork: false, selected: [] });
  assert.equal(model.rows.length, 1);
  const projected = model.rows[0]!;
  assert.deepEqual(projected.cells.place?.value, { kind: 'known', value: 'Brandenburg' });
  assert.deepEqual(projected.cells.country?.value, { kind: 'known', value: 'German Reich · Poland' });
});

test('a raid not read at the hour shown is not recorded, never ended', () => {
  const row = sampleRow({ key: 'r-unread', hour: 10, live: false, status: 'notRead', lastSeen: 20 });
  const source = buildRowSourceFromHead('test:raids:unread', sampleReply([row]), undefined, 50)!;
  const model = raidLogProjection.project(createBoundedReadView({ hour: 50, rows: new Map([['main', source]]), identity: makeIdentity((_, key) => key) }), { artwork: false, selected: [] });
  assert.deepEqual(model.rows[0]!.cells.status?.value, { kind: 'notRecorded', reason: 'Raids were not recorded at this hour' });
});

test('a raid first read ready to launch shows its preparation as not recorded, never as not preparing', () => {
  const row = sampleRow({ key: 'r-ready', hour: 10, status: 'prepared', preparation: null });
  const source = buildRowSourceFromHead('test:raids:ready', sampleReply([row]), undefined, 50)!;
  const model = raidLogProjection.project(createBoundedReadView({ hour: 50, rows: new Map([['main', source]]), identity: makeIdentity((_, key) => key) }), { artwork: false, selected: [] });
  assert.deepEqual(model.rows[0]!.cells.preparation?.value, { kind: 'notRecorded', reason: 'Preparation hours were not recorded' });
});

test('a selected slot untagged raid is listed instead of dropped on the client', () => {
  const untagged = sampleRow({
    key: 'r-untagged',
    hour: 10,
    raider: { tag: null, slot: 3, missing: "The raider's country holds no tag at the raid's hour" },
    victim: { hour: 10, tag: 'ENG', missing: null, source: 'controllerAtObservedRead' },
  });
  const reply = sampleReply([untagged]);
  const source = buildRowSourceFromHead('test:raids:untagged', reply, undefined, 20)!;
  const readView = createBoundedReadView({
    hour: 20,
    rows: new Map([['main', source]]),
    identity: makeIdentity((_kind, key) => (key === 'ENG' ? 'United Kingdom' : key)),
  });
  const model = raidLogProjection.project(readView, { artwork: false, selected: [] });
  assert.equal(model.rows.length, 1);
  const projected = model.rows[0]!;
  assert.equal(projected.key, 'r-untagged');
  assert.deepEqual(projected.cells.country?.flags?.map(flag => flag.tag), ['?', 'ENG']);
  assert.deepEqual(projected.cells.country?.value, {
    kind: 'known',
    value: "The raider's country holds no tag at the raid's hour · United Kingdom",
  });
});

test('an end-dated row shows endedHour and names the end observation raider, never borrowing the start raider', () => {
  const ended = sampleRow({
    key: 'r-ended',
    hour: 10,
    live: false,
    status: 'succeeded',
    raider: { tag: 'DDR', slot: 1, missing: null },
    victim: { hour: 15, tag: 'ROM', missing: null, source: 'completionCountry' },
    preparation: null,
    end: { hour: 20, endedHour: 5, exact: true, source: 'nativeEndDate', raider: 'GER' },
  });
  const endedUnnamed = sampleRow({
    key: 'r-ended-null',
    hour: 12,
    ordinal: '1',
    live: false,
    status: 'cancelled',
    raider: { tag: 'DDR', slot: 1, missing: null },
    victim: { hour: 15, tag: 'ENG', missing: null, source: 'fixedTarget' },
    preparation: null,
    end: { hour: 18, endedHour: 16, exact: false, source: 'observedEndedRead', raider: null },
  });
  const reply = sampleReply([ended, endedUnnamed]);
  const source = buildRowSourceFromHead('test:raids:ended', reply, undefined, 25)!;
  const readView = createBoundedReadView({
    hour: 25,
    rows: new Map([['main', source]]),
    identity: makeIdentity((kind, key, hour) => {
      if (kind === 'country' && key === 'GER') return hour <= 8 ? 'Deutsches Reich' : 'Later Germany';
      if (kind === 'country' && key === 'DDR') return 'East Germany';
      if (kind === 'country' && key === 'ROM') return 'Romania';
      if (kind === 'country' && key === 'ENG') return 'United Kingdom';
      return key;
    }),
  });
  const model = raidLogProjection.project(readView, { artwork: false, selected: [] });
  assert.equal(model.rows.length, 2);

  const first = model.rows[0]!;
  assert.equal(first.rowHour, 5);
  assert.deepEqual(first.cells.date?.value, { kind: 'known', value: 5 });
  assert.deepEqual(first.cells.country?.flags?.map(flag => [flag.tag, Boolean(flag.dimmed)]), [['GER', false], ['ROM', true]]);
  assert.deepEqual(first.cells.country?.value, { kind: 'known', value: 'Deutsches Reich · Romania' });
  assert.equal(first.cells.status?.statusKind, 'succeeded');
  assert.equal(first.muted, false);

  const second = model.rows[1]!;
  assert.equal(second.rowHour, 16);
  assert.deepEqual(second.cells.date?.value, { kind: 'known', value: 16 });
  assert.deepEqual(second.cells.country?.flags?.map(flag => flag.tag), ['?', 'ENG']);
  assert.equal(second.cells.status?.statusKind, 'ended');
  assert.equal(second.muted, true);
});

test('raid projection formats preparation days, nextChangeAt, active count summary, missing spans, and unavailable state', () => {
  const prep = sampleRow({
    key: 'r-prep',
    hour: 100,
    live: true,
    status: 'preparing',
    preparation: { since: 100, hours: 96, perHour: 1, through: null },
  });
  const shortPrep = sampleRow({
    key: 'r-short',
    hour: 100,
    ordinal: '1',
    live: true,
    status: 'preparing',
    preparation: { since: 100, hours: 5, perHour: 1, through: null },
  });
  const ready = sampleRow({
    key: 'r-ready',
    hour: 90,
    ordinal: '2',
    live: true,
    status: 'prepared',
    preparation: null,
  });
  const reply = sampleReply([prep, shortPrep, ready], {
    counts: [{ key: 'active', total: '3' }],
    missing: [{ firstHour: 50, lastHour: 60, reason: 'Unrecorded stretch', before: '3' }],
  });
  const source = buildRowSourceFromHead('test:raids:prep', reply, undefined, 104)!;
  const readView = createBoundedReadView({ hour: 104, rows: new Map([['main', source]]) });
  const model = raidLogProjection.project(readView, { artwork: false, selected: [] });
  assert.equal(model.titleSummary, '3 in preparation');
  assert.equal(model.titleSummaryPartial, 'Count from recorded hours');
  assert.deepEqual(model.rows[0]?.cells.preparation?.value, { kind: 'known', value: '4' });
  assert.equal(model.rows[0]?.cells.preparation?.unitSuffix, 'days preparing');
  assert.deepEqual(model.rows[1]?.cells.preparation?.value, { kind: 'known', value: '9' });
  assert.equal(model.rows[1]?.cells.preparation?.unitSuffix, 'hours preparing');
  assert.equal(model.nextChangeAt, 105, 'short preparation steps on the next hour');
  assert.equal(model.rows[2]?.cells.status?.statusKind, 'waiting');
  assert.deepEqual(model.rows[2]?.cells.status?.value, { kind: 'known', value: 'Ready to launch' });
  assert.ok(model.slots?.some(slot => slot.kind === 'gap' && slot.firstHour === 50 && slot.lastHour === 60));

  const unavailableReply = sampleReply([], { unavailable: 'The content declares no raid type.' });
  const unavailableSource = buildRowSourceFromHead('test:raids:unavailable', unavailableReply, undefined, 104)!;
  const unavailableModel = raidLogProjection.project(
    createBoundedReadView({ hour: 104, rows: new Map([['main', unavailableSource]]) }),
    { artwork: false, selected: [] },
  );
  assert.equal(unavailableModel.emptyKind, 'unavailable');
  assert.equal(unavailableModel.emptyDetail, 'The content declares no raid type.');
});

test('a raid in a log frozen at an earlier hour keeps the preparation it had then, and does not tick with playback', () => {
  const row = sampleRow({ key: 'r-short', hour: 100, preparation: { since: 100, hours: 5, perHour: 1, through: null } });
  const frozen = sampleReply([row], { total: '300' });
  const pages = { 0: { hour: 100, offset: 0, limit: 256, raw: frozen, reply: validateGenericTailReply(frozen, 100) } };
  const live = sampleReply([sampleRow({ key: 'r-new', hour: 104 })]);
  const source = buildRowSourceFromHead('test:raids:frozen', live, undefined, 104, pages)!;
  const model = raidLogProjection.project(createBoundedReadView({ hour: 104, rows: new Map([['main', source]]) }), { artwork: false, selected: [] });
  assert.deepEqual(model.rows[0]?.cells.preparation?.value, { kind: 'known', value: '5' }, 'the hours prepared at the frozen hour, not at the displayed one');
  assert.equal(model.nextChangeAt ?? Infinity, Infinity, 'a frozen row does not wake playback to advance');
  const sameHour = buildRowSourceFromHead('test:raids:frozen-now', frozen, undefined, 100, pages)!;
  const answeredNow = raidLogProjection.project(createBoundedReadView({ hour: 100, rows: new Map([['main', sameHour]]) }), { artwork: false, selected: [] });
  assert.equal(answeredNow.nextChangeAt ?? Infinity, Infinity, 'nor one whose page answered at the hour it froze');
});
