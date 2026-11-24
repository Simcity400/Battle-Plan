import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { OccurrenceReply } from '../../families/table/logOccurrence.ts';
import { raidLogProjection, type RaidOccurrenceRow } from './raidLogProjection.ts';

const day = (month: number, date: number, year = 1941) => engineHour({ year, month, day: date, hour: 12 })!;

test('raid log renders assembling/preparing/ready/succeeded/cancelled rows, dims defeated victim, clips future gaps, and steps prep days', () => {
  const hour = day(9, 15);
  const reply: OccurrenceReply<RaidOccurrenceRow> = {
    branchId: '1',
    total: 9,
    offset: 0,
    counts: { total: 9, live: 5 },
    nextChange: hour + 96,
    missing: [
      { firstHour: day(8, 1), lastHour: day(8, 2), reason: 'raids unread' },
      { firstHour: hour + 24, lastHour: hour + 48, reason: 'future gap must never appear' },
    ],
    rows: [
      {
        id: 'raid:1',
        key: 'raid:1',
        hour: day(9, 11),
        readHour: day(9, 11),
        exact: true,
        live: true,
        raidTypeToken: 'raid_weapon_depots',
        raidName: 'Raid Weapon Depots',
        icon: 'art://raid1',
        raider: { tag: 'SOV', atHour: day(9, 11) },
        victim: { tag: 'MAN', atHour: day(9, 11) },
        placeName: 'Kirin',
        status: 'assembling',
        prepStartedHour: null,
        prepDays: null,
        endedHour: null,
      },
      {
        id: 'raid:2',
        key: 'raid:2',
        hour: day(9, 11),
        readHour: day(9, 11),
        exact: true,
        live: true,
        raidTypeToken: 'facility_strike',
        raidName: 'Facility Strike',
        icon: null,
        raider: { tag: 'ENG', atHour: day(9, 11) },
        victim: { tag: 'GER', atHour: day(9, 11) },
        placeName: 'Vorpommern',
        status: 'preparing',
        prepStartedHour: day(9, 11),
        prepDays: 4,
        endedHour: null,
      },
      {
        id: 'raid:3',
        key: 'raid:3',
        hour: day(8, 24),
        readHour: day(8, 24),
        exact: true,
        live: true,
        raidTypeToken: 'facility_strike',
        raidName: 'Facility Strike',
        icon: null,
        raider: { tag: 'ITA', atHour: day(8, 24) },
        victim: { tag: 'ENG', atHour: day(8, 24) },
        placeName: 'Sussex',
        status: 'ready',
        prepStartedHour: day(8, 24),
        prepDays: 22,
        endedHour: null,
      },
      {
        id: 'raid:4',
        key: 'raid:4',
        hour: day(8, 13),
        readHour: day(8, 13),
        exact: true,
        live: false,
        raidTypeToken: 'oil_field_raid',
        raidName: 'Oil Field Raid',
        icon: null,
        raider: { tag: 'ENG', atHour: day(8, 13) },
        victim: { tag: 'ROM', atHour: day(8, 13) },
        placeName: 'Muntenia',
        status: 'succeeded',
        prepStartedHour: day(7, 10),
        prepDays: null,
        endedHour: day(8, 13),
      },
      {
        id: 'raid:5',
        key: 'raid:5',
        hour: day(7, 30),
        readHour: day(7, 30),
        exact: true,
        live: false,
        raidTypeToken: 'facility_strike',
        raidName: 'Facility Strike',
        icon: null,
        raider: { tag: 'ITA', atHour: day(7, 30) },
        victim: { tag: null, atHour: day(7, 30), missingReason: 'Target owner unread' },
        placeName: null,
        placeMissing: 'Target state not recorded',
        status: 'cancelled',
        prepStartedHour: null,
        prepDays: null,
        endedHour: day(7, 30),
      },
    ],
  };

  const readView = createBoundedReadView({ hour, panels: new Map([['main', reply]]) });
  const model = raidLogProjection.project(readView, { filtered: false, artwork: true });

  assert.equal(model.titleSummary, '5 in preparation');
  assert.equal(model.rows[0]?.cells.preparation?.blank, true, 'assembling leaves preparation blank');
  assert.equal(model.rows[0]?.cells.status?.statusKind, 'waiting');
  assert.equal((model.rows[1]?.cells.preparation?.value as { value: string }).value, '4');
  assert.equal(model.rows[1]?.cells.preparation?.unitSuffix, 'days preparing');
  assert.equal((model.rows[2]?.cells.preparation?.value as { value: string }).value, '22');
  assert.equal(model.rows[2]?.cells.preparation?.unitSuffix, 'days prepared');
  assert.equal(model.rows[2]?.cells.status?.statusKind, 'waiting');
  assert.equal(model.rows[3]?.cells.status?.statusKind, 'succeeded');
  assert.equal(model.rows[3]?.cells.country?.flags?.[1]?.dimmed, true, 'succeeded raid dims victim flag');
  assert.equal(model.rows[4]?.muted, true, 'cancelled raid is muted');
  assert.equal(model.rows[4]?.cells.country?.flags?.[1]?.tag, '?', 'missing victim keeps hatched flag');
  assert.equal(model.rows[4]?.cells.place?.missingFigure, 'Place');
  assert.equal((model.slots ?? []).filter(s => s.kind === 'gap').length, 1, 'future missing stretches are never counted or shown');
  assert.equal(model.nextChangeAt, day(9, 11) + 5 * 24, 'nextChangeAt steps to next preparing day boundary');
});
