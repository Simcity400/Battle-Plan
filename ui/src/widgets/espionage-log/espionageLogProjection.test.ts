import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { DisplayedHour, EntityKind, IdentityProvider, IdentityResult } from '../../families/plan.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { EspionageLogRow, EspionageLogTailReply, EspionageTypeChip } from '../../generated/dashboard';
import {
  espionageLogDeclaration,
  espionageLogProjection,
  espionageTypeFilterItems,
} from './espionageLogProjection.ts';

function sampleRow(overrides: Partial<EspionageLogRow> = {}): EspionageLogRow {
  return {
    key: 'op:100',
    hour: 24 * 10,
    exact: true,
    observed: 24 * 10,
    ordinal: '0',
    live: true,
    status: 'running',
    owner: { tag: 'SOV', slot: 2, missing: null },
    target: { tag: 'GER', slot: 1, missing: null },
    operationToken: 'operation_make_resistance_contacts',
    operationName: 'Make Resistance Contacts',
    operationMissing: null,
    icon: null,
    iconMissing: null,
    startHour: 24 * 10,
    firstSeen: 24 * 5,
    lastSeen: 24 * 20,
    endedHour: null,
    durationDays: 90,
    preparationRatio: 100_000,
    phaseCount: 3,
    currentPhase: 0,
    assignedSlots: 2,
    autoRepeat: false,
    autoCommence: true,
    outcomeMissing: null,
    completion: null,
    ...overrides,
  };
}

function sampleReply(rows: EspionageLogRow[], overrides: Partial<EspionageLogTailReply> = {}): EspionageLogTailReply {
  const liveCount = rows.filter(r => r.live).length;
  return {
    branchId: '1',
    total: String(rows.length),
    offset: '0',
    live: String(liveCount),
    counts: [{ key: 'active', total: String(liveCount) }],
    operationTypes: [],
    rows,
    missing: [],
    nextChange: null,
    unavailable: null,
    warnings: [],
    ...overrides,
  };
}

function makeIdentity(): IdentityProvider {
  return {
    resolve(kind: EntityKind, key: string, atHour: DisplayedHour): IdentityResult {
      return {
        kind,
        key,
        atHour,
        holder: { status: 'absent', reason: 'not set' },
        name: { status: 'ready', value: `${kind}:${key}` },
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    },
  };
}

test('espionageTypeFilterItems hides future chips and disambiguates duplicate names', () => {
  const chips: EspionageTypeChip[] = [
    { token: 'op_a', firstHour: 10, name: 'Infiltrate', icon: null, iconMissing: null },
    { token: 'op_b', firstHour: 20, name: 'Infiltrate', icon: null, iconMissing: null },
    { token: 'op_future', firstHour: 100, name: 'Future Op', icon: null, iconMissing: null },
  ];
  const items = espionageTypeFilterItems(chips, ['op_b'], 50);
  assert.deepEqual(
    items.map(i => ({ value: i.value, label: i.label, checked: i.checked })),
    [
      { value: 'op_a', label: 'Infiltrate (1)', checked: false },
      { value: 'op_b', label: 'Infiltrate (2)', checked: true },
    ],
  );
});

test('espionage projection renders running countdown track, preparing percent, completed duration, and missing outcome', () => {
  const hour = 24 * 41;
  const rows: EspionageLogRow[] = [
    sampleRow({ key: 'op:1', status: 'running', live: true, startHour: 24 * 10, durationDays: 90 }),
    sampleRow({ key: 'op:2', status: 'preparing', live: true, startHour: null, durationDays: null, preparationRatio: 53_400 }),
    sampleRow({
      key: 'op:3',
      status: 'completed',
      live: false,
      durationDays: 157,
      endedHour: 24 * 35,
      completion: {
        completionHour: 24 * 35,
        bonus: true,
        bonusRefused: null,
        detected: false,
        detectedRefused: null,
        description: 'Gained intel bonus',
        descriptionUnrecorded: null,
      },
    }),
    sampleRow({
      key: 'op:4',
      status: 'outcomeNotRecorded',
      live: false,
      durationDays: 138,
      outcomeMissing: 'Completed operations were not recorded whole',
    }),
  ];
  const source = buildRowSourceFromHead('test:espionage', sampleReply(rows), undefined, hour)!;
  const readView = createBoundedReadView({
    hour,
    rows: new Map([['main', source]]),
    identity: makeIdentity(),
  });
  const model = espionageLogProjection.project(readView, { artwork: false, selected: [] });
  assert.equal(model.titleSummary, '2 under way');
  assert.equal(model.rows.length, 4);
  assert.equal(model.nextChangeAt, 24 * 42);

  const running = model.rows[0]!;
  assert.equal(running.cells.progress?.value.kind, 'known');
  assert.equal(running.cells.progress?.value.kind === 'known' ? running.cells.progress.value.value : null, '59');
  assert.equal(running.cells.progress?.unitSuffix, 'days left');
  assert.equal(running.cells.progress?.bar?.fraction.kind, 'known');

  const preparing = model.rows[1]!;
  assert.equal(preparing.cells.progress?.value.kind === 'known' ? preparing.cells.progress.value.value : null, '53%');
  assert.equal(preparing.cells.progress?.unitSuffix, 'prepared');
  assert.equal(preparing.cells.progress?.bar, undefined);

  const completed = model.rows[2]!;
  assert.equal(completed.cells.progress?.value.kind === 'known' ? completed.cells.progress.value.value : null, '157');
  assert.equal(completed.cells.progress?.unitSuffix, 'days');
  assert.equal(completed.cells.status?.value.kind === 'known' ? completed.cells.status.value.value : null, 'Completed');
  const tip = typeof completed.tooltipExtra === 'function' ? (completed.tooltipExtra() ?? []) : (completed.tooltipExtra ?? []);
  assert.ok(tip.some(row => row.label === 'Outcome' && row.value.kind === 'known' && row.value.value === 'Gained intel bonus'));

  const missing = model.rows[3]!;
  assert.equal(missing.cells.status?.value.kind, 'notRecorded');
});

test('espionageLogDeclaration passes operationTypes, countryRole, runningOrder, and resolved slots', () => {
  const plan = espionageLogDeclaration.sources({
    widget: { id: 'w1', kind: 'espionage-log', trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: ['GER'] }, options: {} } as never,
    track: { id: 't1', source: { id: 's1', revision: 'r1' }, branch: { id: '1', firstHour: 0, lastHour: 1000 }, axisOffset: 0 } as never,
    branch: { id: '1', firstHour: 0, lastHour: 1000 } as never,
    following: true,
    hour: 100 as DisplayedHour,
    options: { types: 'operation_capture_cipher', countryRole: 'target', runningOrder: 'preparation', artwork: true },
    countries: { mode: 'filter', keys: ['GER'], tags: ['GER'], selectedSlots: [{ slot: 1 }], selection: { kind: 'tags', tags: ['GER'] }, group: null, label: 'GER', ready: true, unresolved: null },
    countrySubject: null,
    gateSpan: (a: number, b: number) => ({ firstHour: a, lastHour: b }) as never,
  } as never, {} as never);
  const entry = plan.entries[0]!;
  assert.equal(entry.read.shape, 'paged-rows');
  if (entry.read.shape === 'paged-rows') {
    assert.equal(entry.read.tailQuery.product, 'espionage-log-tail');
    assert.deepEqual(entry.read.tailQuery.ask, {
      operationTypes: ['operation_capture_cipher'],
      countryRole: 'target',
      runningOrder: 'preparation',
      countries: [{ slot: 1 }],
    });
  }
});
