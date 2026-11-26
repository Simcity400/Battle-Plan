import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { describe, it } from 'node:test';
import { bucketSamples } from '../../charts/chartSummary.ts';
import { createBoundedReadView, createBoundedSlice, displayedHour, emptySlice } from '../../families/boundedView.ts';
import { buildStaticRequest, hoverAsks } from '../../families/broker.ts';
import type { BoundedSeries, FamilyPlanContext } from '../../families/contract.ts';
import { unitEntries } from '../../families/readUnits.ts';
import { SceneTiles } from '../../families/sceneTiles.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from '../../families/time-chart/hitTest.ts';
import { chartNavStops } from '../../families/time-chart/laneView.ts';
import { formatChartAxisValue, layoutTimeChart } from '../../families/time-chart/layout.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { BoundTrack } from '../../dashboard/model';
import { PlaybackController } from '../../dashboard/playback.ts';
import { keepWanted, type Held } from '../../dashboard/productWindows.ts';
import type { CountryFuelReply, FuelAwardPart, FuelDemandPart, FuelDemandReply } from '../../generated/dashboard';
import { demandHistoryRange } from './fuelConsumption.ts';
import { detailWindows, followedHistoryAnchor, intersectSpans, memberSpansForAliases, pausedChartCutoff } from './fuelDrill.ts';
import {
  buildConsumersTableModel,
  buildFuelAccountingTableModel,
  buildFuelHistoryChartModel,
  buildFuelSeriesChartModel,
  buildRecipientsTableModel,
  buildTopologyTableModel,
  fuelSeriesChartDeclaration,
  fuelSeriesLabelsFromReply,
} from './fuelDetailProjection.ts';

const measure = (text: string) => text.length * 6.5;

function fuelReply(overrides: Partial<CountryFuelReply> = {}): CountryFuelReply {
  return {
    tag: 'GER',
    quantityScale: 32768,
    gainScale: 100000,
    dayHours: 24,
    detailHours: 720,
    consumerNames: ['Army', 'Air Force', 'Navy'],
    gainLabels: [
      { key: 'g0', template: 'Base: {NUM}', arguments: ['NUM'], reason: null },
      { key: 'g1', template: 'States: {NUM}', arguments: ['NUM'], reason: null },
      { key: 'g2', template: 'Oil: {NUM}', arguments: ['NUM'], reason: 'oil table absent' },
      { key: 'g3', template: 'Negative cost: {NUM}', arguments: ['NUM'], reason: null },
    ],
    transferLabels: [
      { key: 't0', template: 'Lend-lease in: {NUM}', arguments: ['NUM'], reason: null },
      { key: 't1', template: 'Lend-lease out: {NUM}', arguments: ['NUM'], reason: null },
      { key: 't2', template: 'Flat: {NUM}', arguments: ['NUM'], reason: null },
    ],
    read: {
      readHour: 120,
      country: 1,
      countryInstance: 1,
      fuelRaw: '3276800',
      capacityRaw: '6553600',
      lendLeaseRaw: '327680',
      dayHour: 120,
      day: {
        gainedRaw: '300000',
        burnedRaw: '163840',
        receivedRaw: ['100000', '50000', '0'],
        otherRaw: '0',
        awards: {
          gainsRaw: ['100000', '200000', '0', '0'],
          ownGainRaw: '300000',
          ownGainQuantityRaw: '98304',
          transfersRaw: ['327680', '0', '0', '0'],
          consumedRaw: '150000',
          receivedRaw: ['100000', '50000', '0'],
        },
      },
      leftOut: null,
      awardsReason: null,
      continuityStartHour: 0,
    },
    missing: null,
    consumption: {
      nativeDailyRaw: ['100000', null, '50000'],
      knownDailyRaw: ['100000', null, '50000'],
      positiveDailyRaw: ['100000', null, '50000'],
      reasons: [null, 'Air wing table missing', null],
    },
    consumptionMissing: null,
    warnings: [],
    ...overrides,
  };
}

describe('fuel detail projections', () => {
  it('builds accounting tables for gain, incoming, burn, consumption, and missing readings', () => {
    const reply = fuelReply();
    const gain = buildFuelAccountingTableModel(120, reply, { tag: 'GER', mode: 'gain', digits: 1, branchId: '1' });
    assert.equal(gain.totalRow?.name, 'Fuel gained (own production)');
    assert.ok(gain.endNote?.includes('oil table absent'));

    const incoming = buildFuelAccountingTableModel(120, reply, { tag: 'GER', mode: 'incoming', digits: 1, branchId: '1' });
    assert.equal(incoming.rows.length, 1);
    assert.equal(incoming.hideHeader, true);

    const missingAwards = buildFuelAccountingTableModel(
      120,
      fuelReply({
        read: {
          ...reply.read!,
          awardsReason: 'Fuel awards table not recorded',
          day: { ...reply.read!.day!, awards: null },
        },
      }),
      { tag: 'GER', mode: 'incoming', digits: 1, branchId: '1' },
    );
    assert.equal(missingAwards.empty, 'Fuel awards table not recorded');
    assert.equal(missingAwards.emptyKind, 'unavailable');

    const burn = buildFuelAccountingTableModel(120, reply, { tag: 'GER', mode: 'burn', digits: 1, branchId: '1' });
    assert.equal(burn.totalRow?.name, 'Fuel burned');
    assert.equal((burn.rows[0]!.meta as { consumer?: number } | undefined)?.consumer, 0);

    const consumption = buildFuelAccountingTableModel(120, reply, { tag: 'GER', mode: 'consumption', digits: 1, branchId: '1' });
    assert.equal(consumption.rows.length, 3);
    const airVal = consumption.rows[1]!.cells.amount?.value;
    assert.ok(airVal && airVal.kind === 'notRecorded');
    assert.equal(airVal.reason, 'Air wing table missing');

    const missing = buildFuelAccountingTableModel(120, fuelReply({ read: null, missing: 'No fuel table' }), { tag: 'GER', mode: 'burn', digits: 1, branchId: '1' });
    assert.equal(missing.empty, 'No fuel table');
    assert.equal(missing.emptyKind, 'unavailable');
  });

  it('builds recipients, consumers (complete vs partial subtotal), and topology tables', () => {
    const recipients: FuelAwardPart[] = [
      { key: 'd:1', kind: 0, reference: '1', owner: 1, host: 'GER', firstHour: 100, lastHour: 120, recordedName: '1st Panzer', nameReason: null, amountRaw: '60000' },
      { key: 'd:2', kind: 0, reference: '2', owner: 1, host: 'GER', firstHour: 100, lastHour: 120, recordedName: null, nameReason: 'Name missing', amountRaw: '15000' },
    ];
    const recTable = buildRecipientsTableModel(120, recipients, 'Army', 100000n, 100000n, 1);
    assert.deepEqual(recTable.rows.map(r => r.key), ['d:1', 'd:2', '__other__']);
    assert.equal(recTable.rows[1]!.muted, true);
    assert.equal(recTable.totalRow?.name, 'Army');

    const consumers: FuelDemandPart[] = [
      { key: 'u:1', kind: 0, reference: '1', parent: 'p1', ordinal: 0, owners: null, wingOwner: null, group: null, topologyReason: null, site: null, recordedName: '2nd Motorized', nameReason: null, amountRaw: '80000', reason: null },
      { key: 'u:2', kind: 0, reference: '2', parent: 'p1', ordinal: 1, owners: null, wingOwner: null, group: null, topologyReason: null, site: null, recordedName: '3rd Cavalry', nameReason: null, amountRaw: null, reason: 'Template missing' },
    ];
    const partial = buildConsumersTableModel(120, consumers, false, 1);
    assert.equal(partial.columns.length, 2, 'incomplete demand drops Share column');
    assert.equal(partial.totalRow?.name, 'Daily demand subtotal');
    assert.equal(partial.totalRow?.cells.amount?.value.kind, 'partial');
    assert.equal(partial.rows[1]!.cells.amount?.value.kind, 'notRecorded');

    const complete = buildConsumersTableModel(120, [consumers[0]!], true, 1);
    assert.equal(complete.columns.length, 3);
    assert.equal(complete.totalRow?.name, 'Daily demand');
    assert.equal(complete.totalRow?.cells.amount?.value.kind, 'known');
    assert.equal(complete.totalRow?.cells.share?.value.kind, 'notApplicable');

    const allUnknown = buildConsumersTableModel(120, [consumers[1]!], false, 1);
    assert.equal(allUnknown.totalRow?.cells.amount?.value.kind, 'notRecorded');

    const topologyData: FuelDemandReply = {
      tag: 'GER',
      asked: 'GER',
      country: 1,
      countryInstance: 1,
      readHour: 120,
      consumption: fuelReply().consumption!,
      parts: consumers,
      commands: [{ reference: 'c1', parent: '0', country: 1, armyGroup: true, members: ['u:1'], children: [], recordedName: 'Heeresgruppe A', nameReason: null }],
      fleets: [{ reference: 'f1', recordedName: 'Kriegsmarine', nameReason: null }],
      sites: [{ list: 0, ordinal: 0, base: '42', containerOwner: 1, capacityRaw: null, demilitarized: null, wings: ['w1', 'w2'], reason: 'Wing count capped' }],
      topologyReason: 'Naval task forces omitted',
      warnings: [],
    };
    const topo = buildTopologyTableModel(120, topologyData);
    assert.deepEqual(topo.rows.map(r => r.key), ['command:c1', 'fleet:f1', 'site:[0,0,0,"42",1]']);
    assert.ok(topo.endNote?.includes('Naval task forces omitted'));
  });

  it('keeps missing reasons per consumer, enforces membership boundaries inside constant runs, and preserves signed exact hover amounts', () => {
    const mixed = buildFuelHistoryChartModel(200, {
      label: 'Consumers',
      series: [
        {
          id: 'c1',
          name: '90. Panzer',
          colorIndex: 0,
          runs: [{ firstHour: 100, lastHour: 180, amountRaw: '22253976', reason: null }],
        },
        {
          id: 'c2',
          name: 'Broken wing',
          colorIndex: 1,
          runs: [{ firstHour: 100, lastHour: 180, amountRaw: null, reason: 'Wing table missing' }],
        },
        {
          id: 'c3',
          name: 'Negative adjustment',
          colorIndex: 2,
          runs: [{ firstHour: 100, lastHour: 180, amountRaw: '-50000', reason: null }],
        },
      ],
      extent: [100, 200],
      viewX: [100, 200],
      selectedHour: 120,
      within: [{ firstHour: 110, lastHour: 150 }],
      minZero: true,
      surface: '#111111',
    });

    assert.equal(mixed.selectedHour, 120);
    assert.equal(mixed.gaps, undefined, 'one missing consumer never adds a chart-wide gap over recorded consumers');
    const hours = mixed.stack!.hours;
    const i100 = hours.indexOf(100);
    const i110 = hours.indexOf(110);
    const i151 = hours.indexOf(151);
    assert.ok(i100 >= 0 && i110 >= 0 && i151 >= 0, 'membership start and end boundaries cut constant runs');
    assert.equal(mixed.stack!.bands[0]!.values[i100], null, 'before membership start is unplotted');
    assert.equal(mixed.stack!.bands[0]!.values[i110], 222.53976, 'inside membership is plotted');
    assert.equal(mixed.stack!.bands[0]!.values[i151], null, 'after membership end is unplotted');
    assert.equal(mixed.stack!.bands[1]!.values[i110], null, 'missing consumer stays null, never zero');
    assert.equal(mixed.stack!.bands[1]!.missingReason?.[i110], 'Wing table missing');
    assert.equal(mixed.stack!.bands[2]!.values[i110], 0, 'negative amount clamps plot geometry at minZero');
    assert.equal(mixed.stack!.bands[2]!.exact?.[i110]?.raw, -50000n, 'exact rational preserves original signed raw');

    const layout = layoutTimeChart(mixed, 600, 220, '12px sans-serif', measure);
    const hit = hitTestTimeChart(mixed, layout, [layout.toX(120), layout.top + 20]);
    assert.equal(hit, 'line:c1@120', 'hit test hits the recorded band instead of collapsing to a gap');
    const tip = deriveTimeChartTooltip(mixed, layout, 'plot:120');
    const items = tip?.payload?.items ?? [];
    assert.equal(items.find(item => item.label === '90. Panzer')?.value, '222.53976');
    assert.equal(items.find(item => item.label === 'Negative adjustment')?.value, '\u22120.5');
    assert.equal(items.find(item => item.label === 'Broken wing')?.missingReason, 'Wing table missing');

    const allMissing = buildFuelHistoryChartModel(200, {
      label: 'Single consumer',
      series: [{
        id: 'd:1',
        name: '1st Panzer',
        colorIndex: 0,
        runs: [
          { firstHour: 100, lastHour: 120, amountRaw: '50000', reason: null },
          { firstHour: 121, lastHour: 140, amountRaw: null, reason: 'Award table missing' },
          { firstHour: 141, lastHour: 180, amountRaw: '75000', reason: null },
        ],
      }],
      extent: [100, 200],
      viewX: [100, 200],
      surface: '#111111',
    });
    assert.deepEqual(allMissing.gaps, [{ firstHour: 121, lastHour: 140, reason: 'Award table missing' }]);
  });

  it('preserves coarse-bucket Fuel peaks and breaks and scopes historical hover to dated slot identity and membership', () => {
    const coarseBuckets = [
      {
        firstHour: 0,
        lastHour: 95,
        firstValue: 0,
        lastValue: 0,
        lastValueHour: 95,
        minHour: 0,
        minValue: 0,
        maxHour: 48,
        maxValue: 99,
        firstBreak: false,
        hasBreak: false,
      },
      {
        firstHour: 96,
        lastHour: 191,
        firstValue: 10,
        lastValue: 20,
        lastValueHour: 191,
        minHour: 96,
        minValue: 10,
        maxHour: 191,
        maxValue: 20,
        firstBreak: false,
        hasBreak: true,
      },
    ];
    const slot7 = countryKey(7);
    const coarseSeries: BoundedSeries = {
      tag: slot7,
      decimals: 0,
      binHours: 96,
      from: 0,
      validInterval: [0, 191],
      runs: emptySlice(),
      buckets: createBoundedSlice(coarseBuckets, coarseBuckets.length),
      missing: emptySlice(),
      straddling: 'none',
    };
    const reply = fuelReply();
    const view = createBoundedReadView({
      hour: displayedHour(200),
      panels: new Map([['fuel', reply]]),
      series: new Map([['positiveArmy', new Map([[slot7, coarseSeries]])]]),
      hovers: new Map([['hover', { hour: 150, data: reply }]]),
    });
    const model = buildFuelSeriesChartModel(view, {
      tag: 'GER',
      historyKey: slot7,
      historySpans: [{ firstHour: 0, lastHour: 140 }],
      mode: 'branches',
      digits: 1,
      extent: [0, 200],
      selectedHour: 48,
      surface: '#111111',
    });
    assert.equal(model.stack?.reduced, true);
    const band = model.stack!.bands[0]!;
    assert.ok(band.values.some(v => v === 99), 'coarse bucket peak at hour 48 is preserved instead of flattened to zero');
    const breakIdx = model.stack!.hours.indexOf(97);
    assert.ok(breakIdx >= 0 && band.values[breakIdx] === null, 'bucket with internal break is not bridged');
    assert.equal(model.hoverDetail, undefined, 'hover outside historySpans membership is suppressed');

    const plan = fuelSeriesChartDeclaration.sources({
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1' },
      options: {
        tag: 'GER',
        historyKey: slot7,
        historySpans: [{ firstHour: 0, lastHour: 140 }],
        mode: 'branches',
        digits: 1,
        extent: [0, 200],
        surface: '#111111',
      },
    } as unknown as FamilyPlanContext, { width: 600, height: 220, following: false, staticSpan: [0, 200] });
    const insideAsks = hoverAsks(plan, 100);
    assert.equal(insideAsks.length, 1);
    assert.equal((insideAsks[0]!.request.params as { country?: unknown }).country, 7, 'hover queries dated slot 7 rather than current tag GER');
    assert.equal(hoverAsks(plan, 150).length, 0, 'hover outside historySpans issues no request');
  });

  it('never re-reads a 720-hour history window per played hour across 100 played hours and anchors followed windows', () => {
    const DETAIL_HOURS = 720;
    const extent: readonly [number, number] = [0, 5000];
    assert.deepEqual(demandHistoryRange(extent, 200, DETAIL_HOURS), [0, 200]);
    const simulate = (startHour: number) => {
      let requests = 0;
      let held: Held<string> = { identity: 'id', data: new Map(), errors: new Map() };
      const [initialStart, initialEnd] = demandHistoryRange(extent, startHour, DETAIL_HOURS);
      const visible: readonly [number, number] = [initialStart, initialEnd];
      for (let step = 0; step < 100; step++) {
        const currentHour = startHour + step;
        const wanted = detailWindows(extent, visible, currentHour, DETAIL_HOURS);
        const wantSet = new Set(wanted);
        const asking = wanted.filter(window => !held.data.has(window));
        requests += asking.length;
        const kept = keepWanted(held, 'id', wantSet);
        const nextData = new Map(kept.data);
        for (const window of asking) nextData.set(window, `w:${window}`);
        held = { ...kept, data: nextData };
      }
      return requests;
    };
    assert.equal(simulate(800), 2, 'within a single 720h window after initial pair: 2 initial windows, 0 refetches across 100 played hours');
    assert.equal(simulate(1400), 3, 'crossing one 720h boundary adds exactly 1 window across 100 played hours, never 100');
  });

  it('keeps missing branch series null with their recorded reason, preserves break/tied bucket extrema, narrows group spans, and surfaces wholly-missing reasons', () => {
    const slot7 = countryKey(7);
    const armyRuns = [{ firstHour: 0, lastHour: 200, value: 60 }];
    const airRuns = [{ firstHour: 0, lastHour: 120, value: 20 }, { firstHour: 181, lastHour: 200, value: 25 }];
    const airMissing = [{ firstHour: 121, lastHour: 180, reason: 'Air wing table missing' }];
    const makeSeries = (runs: typeof armyRuns, missing: typeof airMissing): BoundedSeries => ({
      tag: slot7,
      decimals: 0,
      binHours: 1,
      from: 0,
      validInterval: [0, 200],
      runs: createBoundedSlice(runs, runs.length),
      buckets: emptySlice(),
      missing: createBoundedSlice(missing, missing.length),
      straddling: 'none',
    });
    const view = createBoundedReadView({
      hour: displayedHour(200),
      panels: new Map([['fuel', fuelReply()]]),
      series: new Map([
        ['consumerArmy', new Map([[slot7, makeSeries(armyRuns, [])]])],
        ['consumerAir', new Map([[slot7, makeSeries(airRuns, airMissing)]])],
      ]),
    });
    const model = buildFuelSeriesChartModel(view, {
      tag: 'GER',
      historyKey: slot7,
      historySpans: null,
      mode: 'burn',
      digits: 1,
      extent: [0, 200],
      surface: '#111111',
    });
    const i140 = model.stack!.hours.indexOf(121);
    assert.ok(i140 >= 0);
    assert.equal(model.stack!.bands[0]!.values[i140], 60 / (32768 * 100000));
    assert.equal(model.stack!.bands[1]!.values[i140], null, 'missing Air branch is never coerced to zero');
    assert.equal(model.stack!.bands[1]!.missingReason?.[i140], 'Air wing table missing');
    const layout = layoutTimeChart(model, 600, 220, '12px sans-serif', measure);
    const tip = deriveTimeChartTooltip(model, layout, 'plot:140');
    const items = tip?.payload?.items ?? [];
    assert.ok(items.some(item => item.label === 'Army' && item.value !== undefined));
    assert.equal(items.find(item => item.label === 'Air Force')?.missingReason, 'Air wing table missing');

    const coarseBuckets = [
      {
        firstHour: 0,
        lastHour: 95,
        firstValue: 10,
        lastValue: 15,
        lastValueHour: 95,
        minHour: 20,
        minValue: 2,
        maxHour: 60,
        maxValue: 90,
        firstBreak: false,
        hasBreak: true,
      },
      {
        firstHour: 96,
        lastHour: 191,
        firstValue: 10,
        lastValue: 10,
        lastValueHour: 191,
        minHour: 120,
        minValue: 3,
        maxHour: 120,
        maxValue: 88,
        firstBreak: false,
        hasBreak: false,
      },
    ];
    const coarseSeries: BoundedSeries = {
      tag: slot7,
      decimals: 0,
      binHours: 96,
      from: 0,
      validInterval: [0, 191],
      runs: emptySlice(),
      buckets: createBoundedSlice(coarseBuckets, coarseBuckets.length),
      missing: emptySlice(),
      straddling: 'none',
    };
    const coarseView = createBoundedReadView({
      hour: displayedHour(200),
      panels: new Map([['fuel', fuelReply()]]),
      series: new Map([['positiveArmy', new Map([[slot7, coarseSeries]])]]),
    });
    const coarseModel = buildFuelSeriesChartModel(coarseView, {
      tag: 'GER',
      historyKey: slot7,
      historySpans: null,
      mode: 'branches',
      digits: 1,
      extent: [0, 200],
      surface: '#111111',
    });
    const vals = coarseModel.stack!.bands[0]!.values.filter((v): v is number => v !== null);
    assert.ok(vals.includes(2) && vals.includes(90), 'fuel break bucket preserves min and max');
    assert.ok(vals.includes(3) && vals.includes(88), 'fuel tied-hour bucket preserves min and max');

    const whollyMissing = buildFuelHistoryChartModel(200, {
      label: 'Consumers',
      series: [{
        id: 'w1',
        name: 'Wing 1',
        colorIndex: 1,
        runs: [{ firstHour: 100, lastHour: 200, amountRaw: null, reason: 'Wing table missing' }],
      }],
      extent: [100, 200],
      viewX: [100, 200],
      surface: '#111111',
    });
    assert.equal(whollyMissing.stack, undefined);
    assert.deepEqual(whollyMissing.empty, { message: 'Wing table missing', kind: 'unavailable' });

    assert.deepEqual(
      intersectSpans([{ firstHour: 0, lastHour: 100 }, { firstHour: 150, lastHour: 300 }], [{ firstHour: 50, lastHour: 200 }]),
      [{ firstHour: 50, lastHour: 100 }, { firstHour: 150, lastHour: 200 }],
    );
    assert.deepEqual(
      intersectSpans([{ firstHour: 0, lastHour: 40 }], [{ firstHour: 50, lastHour: 100 }]),
      [],
      'disjoint group membership yields empty span list, not unbounded null',
    );

    const aliasSpans = new Map([
      ['OLD', [{ firstHour: 0, lastHour: 100 }]],
      ['NEW', [{ firstHour: 101, lastHour: 200 }]],
    ]);
    const memberMap = new Map([['OLD', [{ firstHour: 20, lastHour: 80 }]]]);
    assert.deepEqual(
      memberSpansForAliases(aliasSpans, memberMap, 'NEW'),
      [{ firstHour: 20, lastHour: 80 }],
      'missing current alias still preserves earlier alias membership instead of returning unrestricted null',
    );
    assert.deepEqual(
      memberSpansForAliases(new Map([['NEW', [{ firstHour: 101, lastHour: 200 }]]]), memberMap, 'NEW'),
      [],
      'country absent from active membership returns empty spans, never unrestricted null',
    );

    const clippedMissingSeries = makeSeries([], [{ firstHour: 0, lastHour: 200, reason: 'Tile missing' }]);
    const clippedMissingView = createBoundedReadView({
      hour: displayedHour(200),
      panels: new Map([['fuel', fuelReply({ consumerNames: [null, null, null] })]]),
      series: new Map([['positiveArmy', new Map([[slot7, clippedMissingSeries]])]]),
      hovers: new Map([['hover', { hour: 60, data: fuelReply({ consumerNames: [null, null, null] }) }]]),
    });
    const clippedMissingModel = buildFuelSeriesChartModel(clippedMissingView, {
      tag: 'GER',
      historyKey: slot7,
      historySpans: [{ firstHour: 50, lastHour: 120 }],
      mode: 'branches',
      digits: 1,
      extent: [0, 200],
      surface: '#111111',
    });
    assert.deepEqual(clippedMissingModel.gaps, [{ firstHour: 50, lastHour: 120, reason: 'Tile missing' }], 'tile missing spans are clipped to membership');
    assert.equal(clippedMissingModel.legend?.[0]?.label, 'Consumer name unavailable', 'null consumer names never fall back to hardcoded Army/Air/Navy');
    assert.equal(clippedMissingModel.hoverDetail?.items[0]?.label, 'Consumer name unavailable', 'hover with null consumer names never invents Army/Air/Navy');

    const collidingFuelBuckets = [
      {
        firstHour: 0,
        lastHour: 95,
        firstValue: 10,
        lastValue: 15,
        lastValueHour: 80,
        minHour: 0,
        minValue: 3,
        maxHour: 0,
        maxValue: 88,
        firstBreak: false,
        hasBreak: true,
      },
      {
        firstHour: 96,
        lastHour: 191,
        firstValue: 10,
        lastValue: 40,
        lastValueHour: 160,
        minHour: 96,
        minValue: 2,
        maxHour: 96,
        maxValue: 85,
        firstBreak: false,
        hasBreak: false,
      },
    ];
    const collidingFuelSeries: BoundedSeries = {
      tag: slot7,
      decimals: 0,
      binHours: 96,
      from: 0,
      validInterval: [0, 191],
      runs: emptySlice(),
      buckets: createBoundedSlice(collidingFuelBuckets, collidingFuelBuckets.length),
      missing: emptySlice(),
      straddling: 'none',
    };
    const collidingFuelView = createBoundedReadView({
      hour: displayedHour(200),
      panels: new Map([['fuel', fuelReply()]]),
      series: new Map([['positiveArmy', new Map([[slot7, collidingFuelSeries]])]]),
    });
    const collidingFuelModel = buildFuelSeriesChartModel(collidingFuelView, {
      tag: 'GER',
      historyKey: slot7,
      historySpans: null,
      mode: 'branches',
      digits: 1,
      extent: [0, 200],
      surface: '#111111',
    });
    const fBand = collidingFuelModel.stack!.bands[0]!;
    const fVals = fBand.values.filter((v): v is number => v !== null);
    for (const expected of [10, 3, 88, 15, 2, 85, 40]) {
      assert.ok(fVals.includes(expected), `colliding fuel bucket preserves ${expected}`);
    }
    const idx160 = collidingFuelModel.stack!.hours.indexOf(160);
    assert.ok(idx160 >= 0 && fBand.values[idx160] === 40, 'fuel final plateau starts at lastValueHour 160, not lastHour 191');
    const collidingFuelLayout = layoutTimeChart(collidingFuelModel, 600, 220, '12px sans-serif', measure);
    const reducedFuelTip = deriveTimeChartTooltip(collidingFuelModel, collidingFuelLayout, 'plot:1');
    assert.equal(reducedFuelTip?.payload?.items?.[0]?.missingReason, 'This hour is inside a grouped span; its figures come from the read of this hour');

    assert.ok(
      Math.floor(followedHistoryAnchor([0, 5000], 1500, 720) / 720) === 2
        && Math.floor(followedHistoryAnchor([0, 5000], 100, 720) / 720) === 0,
      'followed consumer anchor advances into new 720-hour windows as playback progresses',
    );

    const closingSamples = bucketSamples({
      firstHour: 0,
      lastHour: 5,
      firstValue: 10,
      minHour: 0,
      minValue: 2,
      maxHour: 0,
      maxValue: 20,
      lastValueHour: 0,
      lastValue: 15,
      firstBreak: false,
      hasBreak: false,
    }, 5);
    assert.deepEqual(closingSamples.map(s => s.value), [10, 2, 20, 15], 'closing value is preserved when firstHour and lastValueHour coincide');
    assert.equal(closingSamples.at(-1)?.hour, 5);

    const emptySibling: BoundedSeries = {
      tag: slot7,
      decimals: 0,
      binHours: 1,
      from: 0,
      validInterval: [0, 10],
      runs: emptySlice(),
      buckets: createBoundedSlice([null, null, null], 3),
      missing: emptySlice(),
      straddling: 'none',
    };
    const hourlyArmy = makeSeries([{ firstHour: 0, lastHour: 2, value: 10 }, { firstHour: 3, lastHour: 10, value: 40 }], []);
    const hourlyView = createBoundedReadView({
      hour: displayedHour(10),
      series: new Map([
        ['positiveArmy', new Map([[slot7, hourlyArmy]])],
        ['positiveAir', new Map([[slot7, emptySibling]])],
      ]),
    });
    const hourlyModel = buildFuelSeriesChartModel(hourlyView, {
      tag: 'GER',
      historyKey: slot7,
      historySpans: null,
      mode: 'branches',
      digits: 1,
      extent: [0, 10],
      surface: '#111111',
    });
    assert.equal(hourlyModel.stack?.reduced, undefined, 'empty hourly sibling series never marks stack reduced');
    const hourlyStops = chartNavStops(hourlyModel, layoutTimeChart(hourlyModel, 600, 220, '12px sans-serif', measure));
    assert.ok(hourlyStops.some(([key]) => key === 'plot:0'), 'initial presence hour stays in keyboard stops');

    const chartPlan = fuelSeriesChartDeclaration.sources({
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1' },
      options: {
        tag: 'GER',
        historyKey: slot7,
        historySpans: [{ firstHour: 0, lastHour: 120 }],
        mode: 'branches',
        digits: 1,
        extent: [0, 200],
        surface: '#111111',
      },
    } as unknown as FamilyPlanContext, { width: 600, height: 220, following: false, staticSpan: [0, 200] });
    assert.equal(chartPlan.entries.some(e => e.read.shape === 'point'), false, 'fuel series chart has no required point read at cutoff that could suppress annexed country history');
    assert.ok(
      chartPlan.entries.filter(e => e.read.shape === 'pixel-binned').every(e => e.read.shape === 'pixel-binned' && e.read.headQuery?.product === 'series-state'),
      'pixel-binned fuel series declare series-state headQuery so straddling coarse bins obtain bounded readings',
    );

    const returningClose = bucketSamples({
      firstHour: 0,
      lastHour: 5,
      firstValue: 10,
      minHour: 0,
      minValue: 2,
      maxHour: 0,
      maxValue: 20,
      lastValueHour: 0,
      lastValue: 10,
      firstBreak: false,
      hasBreak: false,
    }, 5);
    assert.deepEqual(returningClose.map(s => s.value), [10, 2, 20, 10], 'closing sample is preserved even when opening and closing values match');
    assert.equal(returningClose.at(-1)?.hour, 5);

    const straddlingSeries: BoundedSeries = {
      tag: slot7,
      decimals: 0,
      binHours: 6,
      from: 0,
      validInterval: [0, 2],
      runs: emptySlice(),
      buckets: emptySlice(),
      missing: emptySlice(),
      straddling: 'loading',
    };
    const straddlingView = createBoundedReadView({
      hour: displayedHour(2),
      series: new Map([['positiveArmy', new Map([[slot7, straddlingSeries]])]]),
    });
    const straddlingModel = buildFuelSeriesChartModel(straddlingView, {
      tag: 'GER',
      historyKey: slot7,
      historySpans: null,
      mode: 'branches',
      digits: 1,
      extent: [0, 100],
      surface: '#111111',
    });
    assert.equal(straddlingModel.empty, undefined, 'straddling loading bin omits empty instead of reporting recorded history as missing');

    const incompleteReply = fuelReply({
      consumption: {
        nativeDailyRaw: ['100000', '50000', null],
        knownDailyRaw: ['100000', '50000', '0'],
        positiveDailyRaw: ['100000', '50000', '0'],
        reasons: [null, null, 'Task force rate unreadable'],
      },
    });
    const incompleteHoverView = createBoundedReadView({
      hour: displayedHour(200),
      panels: new Map([['fuel', incompleteReply]]),
      series: new Map([['positiveNavy', new Map([[slot7, hourlyArmy]])]]),
      hovers: new Map([['hover', { hour: 5, data: incompleteReply }]]),
    });
    const incompleteHoverModel = buildFuelSeriesChartModel(incompleteHoverView, {
      tag: 'GER',
      historyKey: slot7,
      historySpans: null,
      mode: 'branches',
      digits: 1,
      extent: [0, 10],
      surface: '#111111',
    });
    const navyItem = incompleteHoverModel.hoverDetail?.items.find(item => item.label === 'Navy');
    assert.equal(navyItem?.value, '0');
    assert.equal(navyItem?.partialReason, 'Task force rate unreadable', 'incomplete zero demand carries partialReason instead of appearing exact');

    const gapTrack = {
      branch: {
        id: 'b0',
        firstHour: 0,
        lastHour: 200,
        gaps: [{ firstHour: 150, lastHour: 200, reason: 'Paused' }],
      },
    } as unknown as Pick<BoundTrack, 'branch'>;
    assert.equal(pausedChartCutoff(gapTrack, 180, 90, [0, 200]), 149, 'parent seek into trailing gap clamps cutoff to last covered hour while keeping pinned 90');
    assert.equal(pausedChartCutoff(gapTrack, 150, 90, [0, 200]), 149);

    const clock = new PlaybackController();
    clock.setBounds({ firstHour: 0, lastHour: 2000 });
    let crossings = 0;
    const stop = clock.onPublish((prev, next) => {
      if (followedHistoryAnchor([0, 2000], prev.hour, 720) !== followedHistoryAnchor([0, 2000], next.hour, 720)) {
        crossings++;
      }
    });
    try {
      for (let h = 1; h <= 1440; h++) {
        (clock as unknown as { publish: (s: unknown, d?: boolean, f?: boolean) => void }).publish(
          { ...clock.getSnapshot(), position: h, playing: true },
          false,
          true,
        );
      }
    } finally {
      stop();
    }
    assert.equal(crossings, 8, 'onPublish detects quantized window crossings across uninterrupted forward playback');

    const midClose = bucketSamples({
      firstHour: 0,
      lastHour: 17,
      firstValue: 10,
      minHour: 12,
      minValue: 2,
      maxHour: 12,
      maxValue: 20,
      lastValueHour: 12,
      lastValue: 10,
      firstBreak: false,
      hasBreak: false,
    }, 17);
    assert.equal(midClose.at(-1)?.value, 10, 'bucket with extrema timestamped at lastValueHour still closes at 10');
    assert.equal(midClose.at(-1)?.hour, 12);
    assert.ok(midClose.some(p => p.value === 2) && midClose.some(p => p.value === 20));

    const pendingBranchModel = buildFuelSeriesChartModel(
      createBoundedReadView({
        hour: displayedHour(10),
        panels: new Map([['fuel', incompleteReply]]),
        series: new Map([['positiveNavy', new Map([[slot7, makeSeries([{ firstHour: 0, lastHour: 10, value: 0 }], [])]])]]),
      }),
      {
        tag: 'GER',
        historyKey: slot7,
        historySpans: null,
        mode: 'branches',
        digits: 1,
        extent: [0, 10],
        surface: '#111111',
      },
    );
    const pendingTip = deriveTimeChartTooltip(
      pendingBranchModel,
      layoutTimeChart(pendingBranchModel, 600, 220, '12px sans-serif', measure),
      'plot:5',
    );
    assert.equal(pendingTip?.payload?.items?.[0]?.missingReason, 'Loading branch demand…', 'branches tooltip never reports unqualified zero before country-fuel hover arrives');

    const pinnedUnits = unitEntries(chartPlan, false, []);
    assert.equal(pinnedUnits.length, 3, 'pinned fuel series chart emits static series-state units for straddling coarse bins');
    const staticSpec = buildStaticRequest(pinnedUnits[0]!, {
      timeMode: 'range',
      branch: { firstHour: 0, lastHour: 200 },
      window: { firstHour: 0, lastHour: 2 },
      pinHour: 2 as never,
      gate: { recorded: (h: number | null | undefined) => (typeof h === 'number' ? h : null) as never, span: (a: number, b: number) => ({ firstHour: a, lastHour: b }) as never },
    });
    assert.equal(staticSpec?.request.product, 'series-state');

    const outsideBranchModel = buildFuelSeriesChartModel(
      createBoundedReadView({
        hour: displayedHour(20),
        series: new Map([['positiveNavy', new Map([[slot7, makeSeries([{ firstHour: 0, lastHour: 9, value: 12 }], [])]])]]),
      }),
      {
        tag: 'GER',
        historyKey: slot7,
        historySpans: [{ firstHour: 0, lastHour: 9 }],
        mode: 'branches',
        digits: 1,
        extent: [0, 20],
        windowHours: 168,
        surface: '#111111',
      },
    );
    assert.deepEqual(outsideBranchModel.view?.x, [0, 20], 'recent windowHours propagates to Fuel series chart view');
    const outsideTip = deriveTimeChartTooltip(
      outsideBranchModel,
      layoutTimeChart(outsideBranchModel, 600, 220, '12px sans-serif', measure),
      'plot:15',
    );
    assert.equal(outsideTip, null, 'hover outside membership returns no pending branch-demand tooltip');

    const tiles = new SceneTiles();
    const cutSeries = tiles.series(
      {
        widget: 1,
        subs: [],
        pictures: [],
        model: {},
        hour: 6,
        seek: 0,
        speed: 0,
        binding: { generation: 1, surfaces: [{ width: 600, height: 220, dpr: 1 }], offset: 0, pin: 6 },
        inspect: null,
        now: 1000,
      } as never,
      {
        sources: {},
        staticPanels: {
          positiveArmy: {
            branchId: '1',
            measure: 'positiveArmy',
            decimals: 0,
            states: [{
              country: 'GER',
              hour: 6,
              run: { firstHour: 0, lastHour: 17, valueRaw: '50', slot: 1, countryId: 1, startsSegment: true, segmentFirstHour: 0, firstExact: true, lastExact: true },
              drop: null,
              missing: null,
            }],
          },
        },
      } as never,
      {
        tiledBySource: {
          positiveArmy: {
            GER: {
              source: 'positiveArmy',
              tag: 'GER',
              head: 'h',
              decimals: 0,
              binHours: 18,
              from: 0,
              to: 17,
              width: 1,
              buckets: [{
                firstHour: 0,
                lastHour: 17,
                firstValue: 50,
                lastValue: 50,
                lastValueHour: 0,
                minHour: 0,
                minValue: 50,
                maxHour: 0,
                maxValue: 50,
                firstBreak: false,
                hasBreak: false,
              }],
              missing: [],
              parts: [],
            },
          },
        },
        headSourceBySeries: { positiveArmy: { answerName: 'positiveArmy', tags: ['GER'] } },
      },
      displayedHour(6),
    );
    const gerCut = cutSeries?.get('positiveArmy')?.get('GER');
    assert.equal(gerCut?.straddling, 'cut', 'static SeriesState matched by wire state.country tag cuts straddling bin');
    assert.equal(gerCut?.buckets.at(0)?.firstHour, 0, 'cut bucket preserves witnessed run start at hour 0 instead of collapsing to a zero-width point at 6');
    assert.equal(gerCut?.buckets.at(0)?.lastHour, 6);

    const clippedTiles = new SceneTiles();
    const clippedCut = clippedTiles.series(
      {
        widget: 1,
        subs: [],
        pictures: [],
        model: {},
        hour: 6,
        seek: 0,
        speed: 0,
        binding: { generation: 1, surfaces: [{ width: 600, height: 220, dpr: 1 }], offset: 0, pin: 6 },
        inspect: null,
        now: 1000,
      } as never,
      {
        sources: {},
        staticPanels: {
          positiveArmy: {
            branchId: '1',
            measure: 'positiveArmy',
            decimals: 0,
            states: [{
              country: 'GER',
              hour: 6,
              run: { firstHour: 0, lastHour: 17, valueRaw: '50', slot: 1, countryId: 1, startsSegment: true, segmentFirstHour: 0, firstExact: true, lastExact: true },
              drop: null,
              missing: null,
            }],
          },
        },
      } as never,
      {
        tiledBySource: {
          positiveArmy: {
            GER: {
              source: 'positiveArmy',
              tag: 'GER',
              head: 'h',
              decimals: 0,
              binHours: 18,
              from: 0,
              to: 17,
              width: 1,
              buckets: [{
                firstHour: 0,
                lastHour: 17,
                firstValue: 50,
                lastValue: 50,
                lastValueHour: 0,
                minHour: 0,
                minValue: 50,
                maxHour: 0,
                maxValue: 50,
                firstBreak: false,
                hasBreak: false,
              }],
              missing: [],
              parts: [],
            },
          },
        },
        headSourceBySeries: {
          positiveArmy: {
            answerName: 'positiveArmy',
            tags: ['GER'],
            within: { GER: [{ firstHour: 0, lastHour: 2 }, { firstHour: 12, lastHour: 17 }] },
          },
        },
      },
      displayedHour(6),
    );
    const gerClipped = clippedCut?.get('positiveArmy')?.get('GER');
    assert.equal(gerClipped?.buckets.at(0)?.firstHour, 0);
    assert.equal(gerClipped?.buckets.at(0)?.lastHour, 2, 'static head run is clipped to membership so hours 3..6 outside membership never leak');
    assert.equal(gerClipped?.headValue, undefined, 'headValue outside membership is withheld');

    const followedTiles = new SceneTiles();
    const followedAnswer = {
      kind: 'rows' as const,
      canonical: false,
      rows: 1,
      columns: 1,
      views: {
        u8: new Uint8Array([0]),
        u16: new Uint16Array(0),
        u32: new Uint32Array([0xffff_ffff]),
        i32: new Int32Array([50, 0]),
        i64: new BigInt64Array([50n]),
        u64: new BigUint64Array(0),
      },
      keys: 0,
      values: 0,
      hours: 0,
      states: 0,
      reasons: 0,
      names: 0,
      ends: new Uint32Array(0),
      text: 0,
      decoded: [],
    };
    const followedCut = followedTiles.series(
      {
        widget: 1,
        subs: [{ id: 11 }],
        pictures: [{ h: 0, c: 17, parts: [{ version: 1, value: { kind: 'answer', answer: followedAnswer } }] }],
        model: {},
        hour: 6,
        seek: 0,
        speed: 1,
        binding: { generation: 1, surfaces: [{ width: 600, height: 220, dpr: 1 }], offset: 0, pin: null },
        inspect: null,
        now: 1000,
      } as never,
      { sources: { positiveArmy: 11 } } as never,
      {
        tiledBySource: {
          positiveArmy: {
            GER: {
              source: 'positiveArmy',
              tag: 'GER',
              head: 'h',
              decimals: 0,
              binHours: 18,
              from: 0,
              to: 17,
              width: 1,
              buckets: [{
                firstHour: 0,
                lastHour: 17,
                firstValue: 50,
                lastValue: 50,
                lastValueHour: 0,
                minHour: 0,
                minValue: 50,
                maxHour: 0,
                maxValue: 50,
                firstBreak: false,
                hasBreak: false,
              }],
              missing: [],
              parts: [],
            },
          },
        },
        headSourceBySeries: { positiveArmy: { answerName: 'positiveArmy', tags: ['GER'] } },
      },
      displayedHour(6),
    );
    const gerFollowed = followedCut?.get('positiveArmy')?.get('GER');
    assert.equal(gerFollowed?.straddling, 'cut', 'followed fact cuts first coarse bucket');
    assert.equal(gerFollowed?.buckets.at(0)?.firstHour, 0, 'followed fact preserves recorded start at hour 0 instead of collapsing to [6, 6]');
    assert.equal(gerFollowed?.buckets.at(0)?.lastHour, 6);

    const pendingTilesModel = buildFuelSeriesChartModel(
      createBoundedReadView({ hour: displayedHour(100), series: new Map() }),
      {
        tag: 'GER',
        historyKey: slot7,
        historySpans: null,
        mode: 'branches',
        digits: 1,
        extent: [0, 100],
        surface: '#111111',
      },
    );
    assert.equal(pendingTilesModel.empty, undefined, 'unsettled fuel tile sources defer empty-history conclusion while loading');

    let countryReads = 0;
    const manyStates = Array.from({ length: 1500 }, (_, i) => {
      const slot = i + 1;
      return {
        get country() {
          countryReads++;
          return slot;
        },
        hour: 6,
        run: { firstHour: 0, lastHour: 17, valueRaw: String(slot), slot },
        drop: null,
        missing: null,
      };
    });
    const manyLines: Record<string, unknown> = {};
    for (let i = 1; i <= 1500; i++) {
      const key = countryKey(i);
      manyLines[key] = {
        source: 'main',
        tag: key,
        head: 'h',
        decimals: 0,
        binHours: 18,
        from: 0,
        to: 17,
        width: 1,
        buckets: [{ firstHour: 0, lastHour: 17, firstValue: i, lastValue: i, lastValueHour: 0, minHour: 0, minValue: i, maxHour: 0, maxValue: i, firstBreak: false, hasBreak: false }],
        missing: [],
        parts: [],
      };
    }
    const indexedTiles = new SceneTiles();
    indexedTiles.series(
      { widget: 1, subs: [], pictures: [], model: {}, hour: 6, seek: 0, speed: 0, binding: { generation: 1, surfaces: [{ width: 600, height: 220, dpr: 1 }], offset: 0, pin: 6 }, inspect: null, now: 1000 } as never,
      { sources: {}, staticPanels: { main: { decimals: 0, states: manyStates } } } as never,
      { tiledBySource: { main: manyLines as never }, headSourceBySeries: { main: { answerName: 'main', tags: [] } } },
      displayedHour(6),
    );
    assert.ok(countryReads <= 4500, `1,500-country static projection indexes states in O(N), not O(N^2) (reads=${countryReads})`);

    const futureBucketTiles = new SceneTiles();
    const futureBucketCut = futureBucketTiles.series(
      { widget: 1, subs: [], pictures: [], model: {}, hour: 1, seek: 0, speed: 0, binding: { generation: 1, surfaces: [{ width: 600, height: 220, dpr: 1 }], offset: 0, pin: 1 }, inspect: null, now: 1000 } as never,
      { sources: {}, staticPanels: { main: { decimals: 0, states: [{ country: 'GER', hour: 1, run: null, drop: null, missing: 'Gap 0-2' }] } } } as never,
      {
        tiledBySource: {
          main: {
            GER: {
              source: 'main',
              tag: 'GER',
              head: 'h',
              decimals: 0,
              binHours: 18,
              from: 0,
              to: 17,
              width: 1,
              buckets: [{ firstHour: 0, lastHour: 17, firstValue: 20, lastValue: 20, lastValueHour: 0, minHour: 0, minValue: 20, maxHour: 0, maxValue: 20, firstBreak: false, hasBreak: true }],
              missing: [{ firstHour: 0, lastHour: 2, reason: 'Gap 0-2' }],
              parts: [],
            },
          },
        },
        headSourceBySeries: { main: { answerName: 'main', tags: ['GER'] } },
      },
      displayedHour(1),
    );
    const gerFutureBucket = futureBucketCut?.get('main')?.get('GER');
    assert.equal(gerFutureBucket?.buckets.length, 0, 'rounded-down coarse firstHour never leaks a future observation at hour 3 into missing hour 0');

    const gapPreservedSeries: BoundedSeries = {
      tag: slot7,
      decimals: 0,
      binHours: 18,
      from: 0,
      validInterval: [0, 17],
      runs: emptySlice(),
      buckets: createBoundedSlice([{
        firstHour: 0,
        lastHour: 17,
        firstValue: 10,
        lastValue: 50,
        lastValueHour: 12,
        minHour: 0,
        minValue: 10,
        maxHour: 12,
        maxValue: 50,
        firstBreak: false,
        hasBreak: true,
      }], 1),
      missing: createBoundedSlice([{ firstHour: 12, lastHour: 14, reason: 'Paused 12-14' }], 1),
      straddling: 'none',
    };
    const gapPreservedModel = buildFuelSeriesChartModel(
      createBoundedReadView({ hour: displayedHour(17), series: new Map([['positiveArmy', new Map([[slot7, gapPreservedSeries]])]]) }),
      { tag: 'GER', historyKey: slot7, historySpans: null, mode: 'branches', digits: 1, extent: [0, 17], surface: '#111111' },
    );
    const gapBand = gapPreservedModel.stack!.bands[0]!;
    const idx12 = gapPreservedModel.stack!.hours.indexOf(12);
    assert.equal(gapBand.values[idx12], null, 'explicit missing span 12..14 takes precedence over coarse bucket sample at 12');
    assert.equal(gapBand.missingReason?.[idx12], 'Paused 12-14');

    const preciseGainModel = buildFuelSeriesChartModel(
      createBoundedReadView({
        hour: displayedHour(10),
        series: new Map([['gainBase', new Map([[slot7, makeSeries([{ firstHour: 0, lastHour: 10, value: 37_500 }], [])]])]]),
      }),
      { tag: 'GER', historyKey: slot7, historySpans: null, mode: 'gain', digits: 3, extent: [0, 10], surface: '#111111' },
    );
    const preciseLayout = layoutTimeChart(preciseGainModel, 600, 220, '12px sans-serif', measure);
    const preciseTickLabels = preciseLayout.yTicks.map(t => formatChartAxisValue(t, preciseGainModel.yAxis.format, preciseLayout.yTicks));
    assert.ok(new Set(preciseTickLabels).size === preciseLayout.yTicks.length && preciseTickLabels.some(label => label.includes('.')),
      `fractional fuel y-axis ticks stay distinct instead of all rounding to 0 (${preciseTickLabels.join(', ')})`);
    const preciseTip = deriveTimeChartTooltip(
      preciseGainModel,
      preciseLayout,
      'plot:5',
    );
    assert.equal(preciseTip?.payload?.items?.[0]?.value, '0.375', 'hourly gain fallback tooltip preserves selected 3-decimal precision');

    const pendingHistory = buildFuelHistoryChartModel(100, {
      label: 'Recipient receipts',
      series: [{ id: 'r1', name: 'R1', colorIndex: 0, runs: [] }],
      extent: [0, 100],
      viewX: [0, 100],
      pending: true,
      surface: '#111111',
    });
    assert.equal(pendingHistory.empty, undefined, 'loading history window does not display No history recorded in this window');
    const failedHistory = buildFuelHistoryChartModel(100, {
      label: 'Recipient receipts',
      series: [{ id: 'r1', name: 'R1', colorIndex: 0, runs: [] }],
      extent: [0, 100],
      viewX: [0, 100],
      failed: true,
      surface: '#111111',
    });
    assert.equal(failedHistory.empty, undefined, 'failed history window does not display No history recorded in this window');

    const numberedConsumers = fuelSeriesLabelsFromReply(
      fuelReply({
        consumerNames: ['Army 2nd Corps', 'Army 3rd Corps', 'Air'],
      }),
      'burn',
      1,
    );
    assert.equal(numberedConsumers?.['consumer-0'], 'Army 2nd Corps', 'consumer names keep digits intact');
    assert.equal(numberedConsumers?.['consumer-1'], 'Army 3rd Corps', 'consumer names keep digits intact');
    const gainLabels = fuelSeriesLabelsFromReply(
      fuelReply({
        gainLabels: [
          { key: 'g0', template: 'Production: {NUM} from factories', arguments: [], reason: null },
          { key: 'g1', template: 'Oil ({OIL}): {NUM}', arguments: [], reason: null },
          { key: 'g2', template: 'Base: {NUM}', arguments: [], reason: null },
          { key: 'g3', template: 'Negative cost: {NUM}', arguments: [], reason: null },
        ],
      }),
      'gain',
      1,
    );
    assert.equal(gainLabels?.['gain-0'], 'Production from factories', 'localized words after {NUM} are preserved');
    assert.equal(gainLabels?.['gain-1'], 'Oil (?)', '{OIL} token is replaced consistently with the table');

    const clampedConversionModel = buildFuelSeriesChartModel(
      createBoundedReadView({
        hour: displayedHour(10),
        series: new Map([['conversion', new Map([[slot7, makeSeries([{ firstHour: 0, lastHour: 10, value: 0 }], [])]])]]),
      }),
      { tag: 'GER', historyKey: slot7, historySpans: null, mode: 'burn', digits: 1, extent: [0, 10], surface: '#111111' },
    );
    const clampedTip = deriveTimeChartTooltip(
      clampedConversionModel,
      layoutTimeChart(clampedConversionModel, 600, 220, '12px sans-serif', measure),
      'plot:5',
    );
    assert.equal(clampedTip?.payload?.items?.[0]?.value, undefined, 'clamped zero conversion difference does not display exact 0 before signed exact hover arrives');
    assert.equal(clampedTip?.payload?.items?.[0]?.missingReason, 'Signed amount comes from the read of this hour');

    const emptyTopoWithReason = buildTopologyTableModel(100, {
      tag: 'GER',
      asked: 'GER',
      country: 1,
      countryInstance: 1,
      readHour: 100,
      consumption: fuelReply().consumption!,
      parts: [],
      commands: [],
      fleets: [],
      sites: [],
      topologyReason: 'Order of battle not recorded on this day',
      warnings: [],
    });
    assert.equal(emptyTopoWithReason.empty, 'Order of battle not recorded on this day', 'empty topology table surfaces topologyReason instead of hiding it in endNote');
    assert.equal(emptyTopoWithReason.emptyKind, 'unavailable');
    assert.equal(emptyTopoWithReason.endNote, undefined);

    const workerProbe = [
      "import './ui/src/widgets/familyManifest.ts';",
      "import { ensureFamilyProjection } from './ui/src/families/registry.ts';",
      "const checks = [['table', 'fuel-accounting-table'], ['table', 'fuel-detail-table'], ['time-chart', 'fuel-series-chart'], ['time-chart', 'fuel-history-chart']];",
      "await Promise.all(checks.map(([family, kind]) => new Promise((resolve, reject) => {",
      "  const first = ensureFamilyProjection(family, kind, undefined, resolve);",
      "  if (first === 'unknown') reject(new Error(`worker unknown: ${family}/${kind}`));",
      "  else if (first !== 'loading') resolve();",
      "})));",
      "for (const [family, kind] of checks) {",
      "  const loaded = ensureFamilyProjection(family, kind, undefined, () => {});",
      "  if (typeof loaded !== 'object' || loaded.kind !== kind) throw new Error(`worker failed to resolve: ${family}/${kind}`);",
      "}",
    ].join('\n');
    execFileSync(process.execPath, ['--experimental-strip-types', '--input-type=module', '-e', workerProbe], {
      cwd: process.cwd(),
      stdio: 'pipe',
    });
  });
});
