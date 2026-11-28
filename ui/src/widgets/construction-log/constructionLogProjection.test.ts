import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { DisplayedHour, EntityKind, IdentityProvider, IdentityResult } from '../../families/plan.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { ConstructionLogRow, ConstructionLogTailReply, ConstructionTypeChip } from '../../generated/dashboard';
import { constructionLogDeclaration, constructionLogProjection, constructionTypeFilterItems } from './constructionLogProjection.ts';

function sampleRow(overrides: Partial<ConstructionLogRow> = {}): ConstructionLogRow {
  const hour = overrides.hour ?? 100;
  return {
    key: '10',
    hour,
    exact: true,
    observed: hour,
    live: true,
    kind: 'building',
    source: 'census',
    status: 'building',
    owner: { tag: 'USA', slot: 2, missing: null },
    buildingToken: 'industrial_complex',
    buildingName: 'Civilian Factory',
    buildingMissing: null,
    icon: null,
    place: { kind: 'building', state: 358, toState: null, province: null, atHour: hour, name: 'New York' },
    placeMissing: null,
    levels: { built: 2, total: 3, unknown: 0, possible: 0, added: 0, earlierUnknown: false, countersUnread: false },
    levelsMissing: null,
    factories: 15,
    lastFactories: 15,
    factoriesMissing: null,
    createdHour: hour,
    firstRead: hour,
    costRaw: '1080000000',
    baseLevel: null,
    targetLevel: null,
    routeProvinces: null,
    end: null,
    output: null,
    ...overrides,
  };
}

function sampleReply(rows: ConstructionLogRow[], overrides: Partial<ConstructionLogTailReply> = {}): ConstructionLogTailReply {
  const liveCount = rows.filter(r => r.live).length;
  return {
    branchId: '1',
    total: String(rows.length),
    offset: '0',
    live: String(liveCount),
    counts: [{ key: 'active', total: String(liveCount) }],
    buildingTypes: [],
    repairIcon: null,
    factoryCap: 15,
    factoryCapMissing: null,
    costScale: 100_000,
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
        holder: { status: 'absent', reason: 'not set' },
        name: name !== null ? { status: 'ready', value: name } : { status: 'absent', reason: `${kind} ${key} not recorded` },
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    },
  };
}

test('building type chips exclude types first seen after the displayed hour and disambiguate duplicate names', () => {
  const chips: ConstructionTypeChip[] = [
    { token: 'industrial_complex', firstHour: 10, name: 'Factory', icon: null },
    { token: 'mod_factory', firstHour: 12, name: 'Factory', icon: null },
    { token: 'arms_factory', firstHour: 20, name: 'Military Factory', icon: null },
  ];
  const at12 = constructionTypeFilterItems(chips, [], 12);
  assert.deepEqual(
    at12.map(item => [item.value, item.label, item.checked]),
    [
      ['industrial_complex', 'Factory (1)', true],
      ['mod_factory', 'Factory (2)', true],
    ],
  );
  const at20 = constructionTypeFilterItems(chips, ['arms_factory'], 20);
  assert.deepEqual(
    at20.map(item => [item.value, item.label, item.checked]),
    [
      ['industrial_complex', 'Factory (1)', false],
      ['mod_factory', 'Factory (2)', false],
      ['arms_factory', 'Military Factory', true],
    ],
  );
});

test('construction projection formats levels, railway routes, live factories, statuses, and dated renames honestly', () => {
  const rows: ConstructionLogRow[] = [
    sampleRow({
      key: '1',
      hour: 100,
      live: true,
      kind: 'railway',
      buildingToken: 'rail_way',
      buildingName: 'Railways',
      place: { kind: 'railway', state: 10, toState: 11, province: null, atHour: 100, name: 'Illinois', toName: 'Indiana' },
      levels: { built: 8, total: 9, unknown: 0, possible: 0, added: 0, earlierUnknown: false, countersUnread: false },
      factories: 15,
      status: 'building',
    }),
    sampleRow({
      key: '2',
      hour: 98,
      live: true,
      factories: 0,
      status: 'waiting',
      levels: { built: 0, total: 1, unknown: 0, possible: 0, added: 0, earlierUnknown: false, countersUnread: false },
    }),
    sampleRow({
      key: '3',
      hour: 96,
      live: false,
      status: 'built',
      factories: null,
      levels: { built: 0, total: 1, unknown: 0, possible: 1, added: 0, earlierUnknown: false, countersUnread: false },
      end: { kind: 'finished', hour: 96, knownAt: 96, unknown: null },
    }),
    sampleRow({
      key: '4',
      hour: 94,
      live: false,
      source: 'outputAlone',
      status: 'completed',
      owner: { tag: null, slot: 9, missing: 'No tag at first read' },
      factories: null,
      levels: null,
      levelsMissing: 'Levels not recorded: line is known from factory output only',
      end: { kind: 'finished', hour: null, knownAt: 94, unknown: null },
    }),
  ];
  const chips = [
    { token: 'industrial_complex', firstHour: 90, name: 'Civilian Factory', icon: null },
    { token: 'arms_factory', firstHour: 120, name: 'Military Factory', icon: null },
  ];
  const source = buildRowSourceFromHead('test:construction', sampleReply(rows, { buildingTypes: chips }), undefined, 150)!;
  const readView = createBoundedReadView({
    hour: 150,
    rows: new Map([['main', source]]),
    identity: makeIdentity((kind, key) => kind === 'country' && key === 'USA' ? 'United States' : null),
  });
  const model = constructionLogProjection.project(readView, { artwork: false, selected: [] });
  assert.equal(model.rows.length, 4);
  assert.deepEqual(model.kindFilter?.items.map(item => [item.value, item.checked]), [['industrial_complex', true], ['arms_factory', true]], "the head's building types fill the shared filter");
  const chosen = constructionLogProjection.project(readView, { artwork: false, selected: ['arms_factory'] });
  assert.deepEqual(chosen.kindFilter?.items.map(item => item.checked), [false, true]);
  assert.equal(model.titleSummary, undefined, 'the approved construction log has no count beside its title');

  const rail = model.rows[0]!;
  assert.equal(rail.cells.place?.value.kind === 'known' ? rail.cells.place.value.value : null, 'Illinois – Indiana');
  assert.equal(rail.cells.levels?.value.kind === 'known' ? rail.cells.levels.value.value : null, '8 / 9');
  assert.equal(rail.cells.levels?.unitSuffix, 'levels');
  assert.equal(rail.cells.factories?.value.kind === 'known' ? rail.cells.factories.value.value : null, '15 / 15');
  assert.equal(rail.cells.status?.statusKind, 'running');

  const waiting = model.rows[1]!;
  assert.equal(waiting.cells.status?.statusKind, 'waiting');
  assert.equal(waiting.cells.levels?.unitSuffix, 'level');

  const ambiguous = model.rows[2]!;
  assert.equal(ambiguous.cells.levels?.value.kind === 'known' ? ambiguous.cells.levels.value.value : null, '0 or 1');
  assert.equal(ambiguous.cells.factories?.blank, true);
  assert.equal(ambiguous.cells.status?.statusKind, 'finished');

  const untagged = model.rows[3]!;
  assert.equal(untagged.cells.levels?.value.kind, 'notRecorded');
  assert.equal(untagged.cells.country?.flags?.[0]?.art?.status, 'unavailable');
});

test('constructionLogDeclaration sends filters, work kind, queue toggle, order, and resolved country slots', () => {
  const plan = constructionLogDeclaration.sources({
    widget: { id: 'w1', kind: 'construction-log', trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: ['GER'] }, options: {} } as never,
    track: { id: 't1', source: { id: 's1', revision: 'r1' }, branch: { id: '1', firstHour: 10, lastHour: 200 }, axisOffset: 0 } as never,
    branch: { id: '1', firstHour: 10, lastHour: 200 } as never,
    following: true,
    hour: 100 as DisplayedHour,
    options: { types: 'industrial_complex arms_factory', projects: 'repair', queue: true, order: 'start' },
    countries: { mode: 'filter', keys: ['GER'], tags: ['GER'], selectedSlots: [{ slot: 7 }], selection: { kind: 'tags', tags: ['GER'] }, group: null, label: 'GER', ready: true, unresolved: null },
    countrySubject: null,
    gateSpan: (first: number, last: number) => ({ firstHour: first, lastHour: last }) as never,
  } as never, {} as never);
  const entry = plan.entries[0]!;
  assert.equal(entry.read.shape, 'paged-rows');
  if (entry.read.shape === 'paged-rows') {
    assert.equal(entry.read.tailQuery.product, 'construction-log-tail');
    assert.deepEqual(entry.read.tailQuery.ask, {
      buildingTypes: ['industrial_complex', 'arms_factory'],
      placeNames: true,
      projects: 'repair',
      inQueue: true,
      order: 'start',
      countries: [{ slot: 7 }],
    });
  }
});

test('a line shows its factories of the recorded most a line takes, and the count alone when that most is not recorded', () => {
  const row = sampleRow({ key: '1', hour: 100, live: true, factories: 3, status: 'building' });
  const asked: unknown[] = [];
  const project = (overrides: Partial<ConstructionLogTailReply>, artwork = false, line = row) => {
    const source = buildRowSourceFromHead('test:construction:cap', sampleReply([line], overrides), undefined, 150)!;
    const model = constructionLogProjection.project(createBoundedReadView({ hour: 150, rows: new Map([['main', source]]), onArtDemand: handle => { asked.push(handle); } }), { artwork, selected: [] });
    return model.rows[0]!;
  };
  const capped = project({ factoryCap: 20, factoryCapMissing: null }, true);
  assert.deepEqual([capped.cells.factories?.value, capped.cells.factories?.unitSuffix], [{ kind: 'known', value: '3 / 20' }, 'factories'], 'the cap the recording holds, a mod’s included');
  assert.equal(capped.cells.factories?.unitArt !== undefined, true, 'the civilian factory icon comes with the figure');
  assert.ok(asked.some(handle => JSON.stringify(handle) === JSON.stringify({ kind: 'sprite', key: 'GFX_civ_factory', frame: 0 })), 'the game’s civilian factory icon, by its sprite');
  const reason = 'The most factories a construction line takes is not recorded: no recorded content';
  const uncapped = project({ factoryCap: null, factoryCapMissing: reason });
  assert.deepEqual([uncapped.cells.factories?.value, uncapped.cells.factories?.unitSuffix, uncapped.cells.factories?.unitArt], [{ kind: 'known', value: '3' }, 'factories', undefined], 'never a guessed cap');
  const tooltip = (typeof uncapped.tooltipExtra === 'function' ? uncapped.tooltipExtra() : uncapped.tooltipExtra) ?? [];
  assert.deepEqual(tooltip.find(entry => entry.label === 'Most factories')?.value, { kind: 'notRecorded', reason });
  const unread = project({ factoryCap: null, factoryCapMissing: reason }, false, sampleRow({ key: '2', hour: 100, live: true, factories: null, factoriesMissing: 'Assigned factories not recorded: line is known from output only', status: 'building' }));
  const unreadTip = (typeof unread.tooltipExtra === 'function' ? unread.tooltipExtra() : unread.tooltipExtra) ?? [];
  assert.deepEqual(unreadTip.find(entry => entry.label === 'Most factories')?.value, { kind: 'notRecorded', reason }, 'the missing cap is told even when the count is missing too');
  const railway = (factories: number) => project({ factoryCap: 15, factoryCapMissing: null }, false, sampleRow({ key: '3', hour: 100, live: true, kind: 'railway', factories, status: 'building' }));
  assert.deepEqual(railway(4).cells.factories?.value, { kind: 'known', value: '4 / 15' }, 'a railway line working with factories is not in a state that takes none');
  const idle = railway(0);
  assert.deepEqual(idle.cells.factories?.value, { kind: 'known', value: '0' }, 'a railway line with none may be in a state that takes none: no most is guessed');
  const idleTip = (typeof idle.tooltipExtra === 'function' ? idle.tooltipExtra() : idle.tooltipExtra) ?? [];
  assert.equal(idleTip.find(entry => entry.label === 'Most factories')?.value.kind, 'notRecorded');
  assert.deepEqual(project({ factoryCap: 15, factoryCapMissing: null }, false, sampleRow({ key: '4', hour: 100, live: true, kind: 'building', factories: 0, status: 'waiting' })).cells.factories?.value, { kind: 'known', value: '0 / 15' }, 'a building line always takes the define');
});

test('place cells and tooltips use the names and refusals delivered with the page', () => {
  let placeLookups = 0;
  const identity = makeIdentity((kind, key) => {
    if (kind === 'state' || kind === 'province') placeLookups += 1;
    return key;
  });
  const rows: ConstructionLogRow[] = [
    sampleRow({ key: 'p-state', place: { kind: 'building', state: 77, toState: null, province: null, atHour: 100, name: 'North Harbour' } }),
    sampleRow({ key: 'p-province', place: { kind: 'building', state: null, toState: null, province: 30, atHour: 99, name: 'Norway' } }),
    sampleRow({ key: 'p-rail-mixed', kind: 'railway', place: { kind: 'railway', state: 10, toState: 88, province: null, atHour: 100, name: 'Illinois', toNameMissing: 'State 88 was not recorded' } }),
    sampleRow({ key: 'p-rail-half-null', kind: 'railway', place: { kind: 'railway', state: 10, toState: null, province: null, atHour: 100, name: 'Illinois' } }),
  ];
  const source = buildRowSourceFromHead('test:construction:pending-place', sampleReply(rows), undefined, 100)!;
  const model = constructionLogProjection.project(
    createBoundedReadView({ hour: 100, rows: new Map([['main', source]]), identity }),
    { artwork: false, selected: [] },
  );
  const tipOf = (idx: number) => {
    const extra = model.rows[idx]!.tooltipExtra;
    return (typeof extra === 'function' ? extra() : extra) ?? [];
  };
  assert.deepEqual(model.rows[0]!.cells.place?.value, { kind: 'known', value: 'North Harbour' });
  assert.deepEqual(tipOf(0).find(item => item.label === 'Place')?.value, { kind: 'known', value: 'North Harbour' });
  assert.deepEqual(model.rows[1]!.cells.place?.value, { kind: 'known', value: 'Norway' });
  assert.deepEqual(tipOf(1).find(item => item.label === 'Place')?.value, { kind: 'known', value: 'Norway' });
  assert.deepEqual(model.rows[2]!.cells.place?.value, { kind: 'partial', value: 'Illinois', reason: 'State 88 was not recorded' });
  assert.deepEqual(tipOf(2).find(item => item.label === 'Place')?.value, { kind: 'partial', value: 'Illinois', reason: 'State 88 was not recorded' });
  assert.deepEqual(model.rows[3]!.cells.place?.value, { kind: 'partial', value: 'Illinois', reason: 'Railway endpoint was not recorded' });
  assert.deepEqual(tipOf(3).find(item => item.label === 'Place')?.value, { kind: 'partial', value: 'Illinois', reason: 'Railway endpoint was not recorded' });
  assert.equal(placeLookups, 0);
});
