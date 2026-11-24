import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { OccurrenceReply } from '../../families/table/logOccurrence.ts';
import { espionageLogProjection, type EspionageDetailSubject, type EspionageOccurrenceRow } from './espionageLogProjection.ts';

const day = (month: number, date: number, year = 1941) => engineHour({ year, month, day: date, hour: 12 })!;

test('espionage log renders running timers, preparation percentages, finished durations, outcome refusals, and detail meta', () => {
  const hour = day(9, 15);
  const reply: OccurrenceReply<EspionageOccurrenceRow> = {
    branchId: '1',
    total: 12,
    offset: 0,
    counts: { total: 12, live: 3 },
    nextChange: hour + 72,
    missing: [],
    rows: [
      {
        id: 'op:1',
        key: 'op:1',
        hour: day(7, 19),
        readHour: day(7, 19),
        exact: true,
        live: true,
        operationToken: 'operation_make_resistance_contacts',
        operationName: 'Make Resistance Contacts',
        icon: 'art://op1',
        owner: { tag: 'SOV', atHour: day(7, 19) },
        target: { tag: 'GER', atHour: day(7, 19) },
        status: 'running',
        startedHour: day(7, 19),
        endedHour: null,
        durationDays: 117,
        elapsedDays: 58,
        preparationFraction: null,
        assignedSlots: 2,
      },
      {
        id: 'op:2',
        key: 'op:2',
        hour: day(9, 15),
        readHour: day(9, 15),
        exact: true,
        live: true,
        operationToken: 'operation_infiltrate_armed_forces_army',
        operationName: 'Infiltrate Army',
        icon: null,
        owner: { tag: 'GER', atHour: day(9, 15) },
        target: { tag: 'SOV', atHour: day(9, 15) },
        status: 'preparing',
        startedHour: day(9, 15),
        endedHour: null,
        durationDays: null,
        elapsedDays: null,
        preparationFraction: 0.53,
      },
      {
        id: 'op:3',
        key: 'op:3',
        hour: day(8, 29),
        readHour: day(8, 29),
        exact: false,
        live: false,
        operationToken: 'operation_infiltrate_armed_forces_navy',
        operationName: 'Infiltrate Navy',
        icon: null,
        owner: { tag: 'ITA', atHour: day(8, 29) },
        target: { tag: 'ENG', atHour: day(8, 29) },
        status: 'ended',
        outcomeMissing: 'Operation completion log was not recorded in this stretch',
        startedHour: day(4, 13),
        endedHour: day(8, 29),
        durationDays: 138,
        elapsedDays: null,
        preparationFraction: null,
      },
      {
        id: 'op:4',
        key: 'op:4',
        hour: day(8, 14),
        readHour: day(8, 14),
        exact: true,
        live: false,
        operationToken: 'operation_infiltrate_civilian',
        operationName: 'Infiltrate Civilian Administration',
        icon: null,
        owner: { tag: 'GER', atHour: day(8, 14) },
        target: { tag: 'ENG', atHour: day(8, 14) },
        status: 'succeeded',
        startedHour: day(3, 10),
        endedHour: day(8, 14),
        durationDays: 157,
        elapsedDays: null,
        preparationFraction: null,
      },
    ],
  };

  const readView = createBoundedReadView({ hour, panels: new Map([['main', reply]]) });
  const model = espionageLogProjection.project(readView, { filtered: false, artwork: true });

  assert.equal(model.titleSummary, '3 under way');
  assert.equal((model.rows[0]?.cells.progress?.value as { value: string }).value, '59');
  assert.equal(model.rows[0]?.cells.progress?.unitSuffix, 'days left');
  assert.ok(model.rows[0]?.cells.progress?.bar);
  assert.equal((model.rows[1]?.cells.progress?.value as { value: string }).value, '53%');
  assert.equal(model.rows[1]?.cells.progress?.unitSuffix, 'prepared');
  assert.equal(model.rows[2]?.cells.status?.value.kind, 'notRecorded');
  assert.equal(model.rows[2]?.cells.status?.missingFigure, 'Outcome');
  assert.equal((model.rows[2]?.cells.progress?.value as { value: string }).value, '138');
  assert.equal(model.rows[2]?.cells.progress?.unitSuffix, 'days');
  assert.equal(model.rows[3]?.cells.status?.statusKind, 'succeeded');
  const meta = model.rows[0]?.meta as EspionageDetailSubject;
  assert.equal(meta.id, 'op:1');
  assert.equal(meta.asOfHour, hour);
  assert.equal(model.nextChangeAt, day(7, 19) + 59 * 24, 'nextChangeAt steps to the next timer day boundary');
});
