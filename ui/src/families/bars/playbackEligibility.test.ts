import assert from 'node:assert/strict';
import test from 'node:test';
import { airForceSizeRaceProjection } from '../../widgets/air-force-size/airForceSizeRaceProjection.ts';
import { armySizeRaceProjection } from '../../widgets/army-size/armySizeRaceProjection.ts';
import { MEASURES } from '../../widgets/army-size/armySizeMeta.ts';
import { casualtiesRaceProjection } from '../../widgets/casualties/casualtiesProjection.ts';
import { equipmentProducedRaceProjection } from '../../widgets/equipment-produced/equipmentProducedRaceProjection.ts';
import { factionStrengthRaceProjection } from '../../widgets/faction-strength/factionStrengthRaceProjection.ts';
import { COUNT_MODES } from '../../widgets/factories/factoriesData.ts';
import { factoriesProjection } from '../../widgets/factories/factoriesProjection.ts';
import { SCORE_PARTS } from '../../widgets/game-score/gameScoreData.ts';
import { gameScoreRaceProjection } from '../../widgets/game-score/gameScoreRaceProjection.ts';
import { navalLossesRaceProjection } from '../../widgets/naval-losses/navalLossesRaceProjection.ts';
import { navySizeRaceProjection } from '../../widgets/navy-size/navySizeRaceProjection.ts';
import { nuclearArsenalRaceProjection } from '../../widgets/nuclear-arsenal/nuclearArsenalRaceProjection.ts';
import { occupationTrendRaceProjection } from '../../widgets/occupation-trend/occupationTrendRaceProjection.ts';
import { territoryRaceProjection } from '../../widgets/territory/territoryRaceProjection.ts';
import { worldTensionRaceProjection } from '../../widgets/world-tension/worldTensionRaceProjection.ts';
import { createBoundedReadView, displayedHour } from '../boundedView.ts';
import type { FactionTotalsRaceOptions } from './factionTotalsRace.ts';
import { isSingleValueOrdered, type BarsProjection } from './model.ts';

const hour = displayedHour(100);
const settings = { top: null, group: false, surface: '#1e1e1e' };
const booleans = [false, true];
const counts = [0, 40];
const factions: FactionTotalsRaceOptions = {
  ...settings, kind: 'territory', measure: 'controlledStates', measureLabel: 'States',
  scaleUnit: 'States', aggregated: false, groupLabel: 'Faction',
};

function rank(value: number, ...measures: string[]) {
  return {
    branchId: '1', hour, from: hour, until: hour, refused: null,
    measures: measures.map(name => ({ name, decimals: 0 })),
    rows: [{
      country: 'GER',
      readings: measures.map(() => ({
        country: 'GER', hour, drop: null, missing: null,
        run: { valueRaw: String(value), firstHour: hour, lastHour: hour, slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: 0, firstExact: true, lastExact: true },
      })),
    }],
  };
}

function forces(value: number) {
  return {
    firstHour: hour, lastHour: hour, missing: null, warnings: [],
    parts: [{ key: 'p', name: 'Part', unresolved: false }], ordinals: [0],
    rows: [{ country: 'GER', slot: null, tag: 'GER', total: value, referenceTotal: 60, missing: null, values: [value], partsMissing: null }],
  };
}

function factionReply(value: number, options: FactionTotalsRaceOptions) {
  return {
    hour, kind: options.kind, measure: options.measure, missing: null, warnings: [],
    rows: [{ id: '10', members: ['GER'], memberCountries: [1], total: value, missing: null }],
  };
}

function check<O>(projection: BarsProjection<O>, options: O, expected: boolean, panel: (value: number) => unknown, source = 'main') {
  const context = `${projection.kind} ${JSON.stringify(options)}`;
  assert.equal(projection.singleValueBars?.(options), expected, context);
  if (!expected) return;
  for (const amount of counts) {
    const view = createBoundedReadView({ hour, panels: new Map([[source, panel(amount)]]) });
    const model = projection.project(view, options);
    assert.ok(model.rows.some(row => row.kind === 'data'), context);
    assert.ok(isSingleValueOrdered(model), context);
  }
  for (const view of [
    createBoundedReadView({ hour }),
    createBoundedReadView({ hour, refused: new Map([[source, 'Unavailable']]) }),
  ]) assert.ok(isSingleValueOrdered(projection.project(view, options)), context);
}

test('splits and multiple measures exclude a binding even when it has no positive segments yet', () => {
  for (const split of booleans) {
    for (const { value: mode } of COUNT_MODES) check(factoriesProjection, {
      ...settings, mode, split, modeLabel: 'Factories', damagedSource: 'damaged7',
      parts: [{ id: 'p', label: 'Part', source: `${mode}1`, color: '#ff8800' }],
    }, !split && mode !== 'built', amount => rank(amount, `${mode}7`, `${mode}1`, 'damaged7'));
    for (const by of SCORE_PARTS) check(gameScoreRaceProjection, { ...settings, by, split },
      !split || by !== 'total', amount => rank(amount, by));
    for (const measure of ['planes', 'wings'] as const) for (const ownership of ['roster', 'effective'] as const) {
      check(airForceSizeRaceProjection, { ...settings, measure, ownership, split }, !split, amount => rank(amount, 'measure'));
    }
    for (const { value: measure } of MEASURES) for (const countFor of ['owner', 'host'] as const) {
      check(armySizeRaceProjection, { ...settings, measure, countFor, split }, !split || measure !== 'battalions', amount => rank(amount, 'measure'));
    }
    for (const measure of ['ships', 'strength'] as const) for (const ownership of ['roster', 'logical'] as const) for (const grouping of ['class', 'type'] as const) {
      check(navySizeRaceProjection, { ...settings, measure, ownership, grouping, split }, !split, amount => measure === 'ships' ? rank(amount, 'measure') : forces(amount));
    }
    for (const kind of ['ships', 'convoys', 'both'] as const) for (const attribution of ['lost', 'sunkBy'] as const) for (const period of ['month', 'running'] as const) {
      check(navalLossesRaceProjection, { ...settings, kind, attribution, period, split }, !split, forces);
    }
    check(nuclearArsenalRaceProjection, { ...settings, split }, !split, amount => ({
      types: [{ ordinal: 0, token: 'type', name: 'Type' }], missing: null, warnings: [],
      countries: [{ tag: 'GER', countryId: 1, slot: null, total: amount, counts: [{ ordinal: 0, value: amount, ready: 0 }] }],
    }));
  }
});

test('race projections that guarantee a single value preserve their form for recorded and missing facts', () => {
  for (const view of ['race', 'butterfly'] as const) for (const includeUnknown of booleans) for (const group of booleans) {
    check(casualtiesRaceProjection, { view, includeUnknown, group }, view === 'race', amount => rank(amount, 'taken', 'unknown', 'partial'), 'rank');
  }
  for (const count of ['units', 'ic'] as const) for (const measure of ['total', 'day', 'week', 'month'] as const) {
    check(equipmentProducedRaceProjection, { ...settings, count, measure, equipmentLabel: 'Equipment', measureLabel: 'Production', noData: null }, true, amount => ({
      branchId: '1', hour, firstHour: hour, lastHour: hour, count, measure, decimals: 0, types: ['equipment'],
      rows: [{ country: 1, counted: 1, tag: 'GER', value: amount, unknown: null }], readHour: hour,
      observationHour: 1, missing: null, warnings: [],
    }));
  }
  for (const aggregated of booleans) for (const kind of ['territory', 'country-factories'] as const) {
    const options = { ...factions, kind, aggregated, measure: kind === 'territory' ? 'controlledStates' : 'free7' };
    check(factionStrengthRaceProjection, options, true, amount => factionReply(amount, options));
  }
  for (const mode of ['countries', 'factions'] as const) for (const measure of ['states', 'provinces', 'victoryPoints', 'area'] as const) for (const rule of ['controlled', 'owned', 'occupied'] as const) {
    const wire = rule + measure[0]!.toUpperCase() + measure.slice(1);
    const options = { ...factions, measure: wire };
    check(territoryRaceProjection, { mode, measure, rule, measureLabel: measure, scaleUnit: measure, group: false, factions: options },
      true, amount => mode === 'countries' ? rank(amount, wire) : factionReply(amount, options));
  }
  for (const rankBy of ['resistance', 'compliance'] as const) for (const digits of [0, 2, 5]) {
    check(occupationTrendRaceProjection, { ...settings, rank: rankBy, digits }, true, amount => rank(amount, rankBy));
  }
  check(worldTensionRaceProjection, settings, true, amount => rank(amount, 'contribution'));
});
