import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { createBoundedReadView, FutureDataError } from '../../families/boundedView.ts';
import type { OccurrenceReply } from '../../families/table/logOccurrence.ts';
import { constructionLogProjection, type ConstructionOccurrenceRow } from './constructionLogProjection.ts';

const day = (month: number, date: number, year = 1941) => engineHour({ year, month, day: date, hour: 12 })!;

test('construction log renders product occurrence window verbatim with levels, recorded factory max, mod statuses, and bounded gaps', () => {
  const hour = day(9, 15);
  const reply: OccurrenceReply<ConstructionOccurrenceRow> = {
    branchId: '1',
    total: 42,
    offset: 0,
    counts: { total: 42, live: 3, partial: null },
    nextChange: hour + 12,
    availableKinds: [
      { key: 'supply_node', label: 'Supply Hub', count: 5 },
      { key: 'mod_orbital_lift', label: 'Orbital Lift', count: 1 },
    ],
    missing: [
      { firstHour: day(9, 14) - 6, lastHour: day(9, 14), reason: 'construction queue unread' },
      { firstHour: hour + 48, lastHour: hour + 72, reason: 'future stretch must be clipped' },
    ],
    rows: [
      {
        id: 'c:1',
        key: 'c:1',
        hour: day(9, 3),
        readHour: day(9, 3),
        exact: true,
        live: true,
        buildingToken: 'rail_way',
        buildingName: 'Railways',
        icon: 'art://railways',
        repair: false,
        country: { tag: 'USA', atHour: day(9, 3) },
        placeName: 'Illinois – Indiana',
        builtLevels: 8,
        orderedLevels: 9,
        levelProgressFraction: 0.62,
        factoriesAssigned: 15,
        factoriesMax: 20,
        status: 'building',
      },
      {
        id: 'c:2',
        key: 'c:2',
        hour: day(9, 15),
        readHour: day(9, 15),
        exact: false,
        live: false,
        buildingToken: 'arms_factory',
        buildingName: 'Military Factory',
        icon: null,
        repair: false,
        country: { tag: 'BHU', atHour: day(9, 15) },
        placeName: 'Bhutan',
        builtLevels: null,
        orderedLevels: null,
        possibleRange: [0, 1],
        factoriesAssigned: null,
        factoriesMax: null,
        status: 'built',
      },
      {
        id: 'c:3',
        key: 'c:3',
        hour: day(9, 13),
        readHour: day(9, 13),
        exact: true,
        live: false,
        buildingToken: 'bunker',
        buildingName: 'Land Fort',
        icon: null,
        repair: false,
        country: { tag: 'SOV', atHour: day(9, 13) },
        placeName: null,
        placeMissing: 'State id 999 not in map',
        builtLevels: null,
        orderedLevels: null,
        levelsMissing: 'Queue census missed this hour',
        factoriesAssigned: null,
        factoriesMax: null,
        status: 'mod_committee_review',
        statusLabel: 'In committee',
      },
      {
        id: 'c:4',
        key: 'c:4',
        hour: day(9, 12),
        readHour: day(9, 12),
        exact: true,
        live: true,
        buildingToken: 'mod_orbital_lift',
        buildingName: 'Orbital Lift',
        icon: null,
        repair: false,
        country: { tag: 'GER', atHour: day(9, 12) },
        placeName: 'Brandenburg',
        builtLevels: 0,
        orderedLevels: 1,
        factoriesAssigned: 3,
        factoriesMax: null,
        status: 'waiting',
      },
    ],
  };

  const readView = createBoundedReadView({
    hour,
    panels: new Map([['main', reply]]),
    art: { resolve: () => ({ status: 'ready', assetId: 'civ', url: 'art://civ', origin: 'vanilla', file: 'civ.dds', frame: 0, provenance: 'recorded' }) as never },
  });
  const model = constructionLogProjection.project(readView, { filtered: false, artwork: true });

  assert.equal(model.titleSummary, '3 building');
  assert.equal(model.titleSummaryPartial, 'Count from recorded hours');
  assert.equal(model.nextChangeAt, hour + 12);
  const gapSlots = (model.slots ?? []).filter(s => s.kind === 'gap');
  assert.equal(gapSlots.length, 1, 'future missing stretches are excluded');
  assert.equal(model.rows[0]?.cells.levels?.value.kind, 'known');
  assert.equal((model.rows[0]?.cells.levels?.value as { value: string }).value, '8 / 9');
  assert.equal((model.rows[0]?.cells.factories?.value as { value: string }).value, '15 / 20', 'uses recorded mod factory max, never hard-coded 15');
  assert.equal(model.rows[0]?.cells.factories?.unitIcon, 'art://civ');
  assert.equal((model.rows[1]?.cells.levels?.value as { value: string }).value, '0 or 1');
  assert.equal(model.rows[1]?.cells.levels?.unitSuffix, 'level');
  assert.equal(model.rows[1]?.cells.factories?.blank, true);
  assert.equal(model.rows[2]?.cells.levels?.value.kind, 'notRecorded');
  assert.equal(model.rows[2]?.cells.levels?.missingFigure, 'Levels');
  assert.equal(model.rows[2]?.cells.place?.missingFigure, 'Place');
  assert.equal((model.rows[2]?.cells.status?.value as { value: string }).value, 'In committee', 'unknown mod status uses recorded label with neutral dot');
  assert.equal(model.rows[2]?.cells.status?.statusKind, 'finished');
  assert.equal((model.rows[3]?.cells.factories?.value as { value: string }).value, '3', 'unrecorded max shows assigned factories alone');
  assert.equal(model.rows[3]?.cells.factories?.unitSuffix, 'factories');
});

test('construction log refuses any occurrence row past the displayed hour', () => {
  const hour = day(9, 15);
  const futureReply: OccurrenceReply<ConstructionOccurrenceRow> = {
    branchId: '1',
    total: 1,
    offset: 0,
    counts: { total: 1, live: 1 },
    nextChange: null,
    missing: [],
    rows: [
      {
        id: 'c:future',
        key: 'c:future',
        hour: hour + 1,
        readHour: hour + 1,
        exact: true,
        live: true,
        buildingToken: 'rail_way',
        buildingName: 'Railways',
        icon: null,
        repair: false,
        country: { tag: 'USA', atHour: hour + 1 },
        placeName: 'Texas',
        builtLevels: 1,
        orderedLevels: 2,
        factoriesAssigned: 5,
        factoriesMax: 15,
        status: 'building',
      },
    ],
  };
  const readView = createBoundedReadView({ hour, panels: new Map([['main', futureReply]]) });
  assert.throws(() => constructionLogProjection.project(readView, { filtered: false, artwork: false }), FutureDataError);
});
