import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry, type FamilyViewModule } from '../../families/manifest.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import { LINES_VIEW_OPTIONS } from '../../dashboard/lineChartOptions.ts';
import { OPTIONS as AIR_KILLS_LOSSES_OPTIONS } from '../air-kills-losses/definition.ts';
import { OPTIONS as ARMY_SIZE_OPTIONS } from '../army-size/definition.ts';
import { OPTIONS as DOCKYARD_OPTIONS } from '../dockyard-spending/definition.ts';
import { TIME_OPTIONS as STOCKPILE_TIME_OPTIONS } from '../equipment-stockpile/definition.ts';
import { TIME_OPTIONS as RESOURCE_TIME_OPTIONS } from '../resource-balance/definition.ts';
import { OPTIONS as FACTORY_OPTIONS } from '../military-factory-spending/definition.ts';
import { OPTIONS as FACTION_STRENGTH_OPTIONS } from '../faction-strength/definition.ts';
import { OPTIONS as GAME_SCORE_OPTIONS } from '../game-score/definition.ts';
import { OPTIONS as NAVAL_LOSSES_OPTIONS } from '../naval-losses/definition.ts';
import { OPTIONS as NUCLEAR_ARSENAL_OPTIONS } from '../nuclear-arsenal/definition.ts';
import { OPTIONS as OCCUPATION_RESERVE_OPTIONS } from '../occupation-reserve-manpower/definition.ts';
import { OPTIONS as RAID_TALLY_OPTIONS } from '../raid-tally/definition.ts';
import { OPTIONS as WORLD_TENSION_OPTIONS, TENSION_CHART_OPTIONS } from '../world-tension/definition.ts';

const AIR_KILLS_LOSSES_LINES_OPTIONS = [
  ...AIR_KILLS_LOSSES_OPTIONS
    .filter(field => ['view', 'measure', 'regions', 'causes', 'endLabels', 'yMin', 'yMax', 'log'].includes(field.key))
    .map(field => ({ ...field, visibleWhen: undefined })),
  ...LINES_VIEW_OPTIONS,
];
const ARMY_SIZE_BREAKDOWN_OPTIONS = ARMY_SIZE_OPTIONS.filter(field => ['view', 'measure', 'countFor', 'log', 'yMin', 'yMax'].includes(field.key));
const DOCKYARD_MONTHS_OPTIONS = DOCKYARD_OPTIONS.filter(field => field.key === 'view' || field.key === 'icDecimals');
const FACTORY_MONTHS_OPTIONS = FACTORY_OPTIONS.filter(field => field.key === 'equipment' || field.key === 'view' || field.key === 'icDecimals');
const FACTION_STRENGTH_LINES_OPTIONS = [
  ...FACTION_STRENGTH_OPTIONS.filter(field => ['view', 'measure', 'count', 'territoryMeasure', 'rule', 'endLabels', 'yMin', 'yMax', 'log'].includes(field.key)),
  ...LINES_VIEW_OPTIONS,
];
const GAME_SCORE_LINES_OPTIONS = [
  ...GAME_SCORE_OPTIONS.filter(field => ['view', 'topMeasure', 'endLabels', 'yMin', 'yMax', 'log'].includes(field.key)),
  ...LINES_VIEW_OPTIONS,
];
const NAVAL_LOSSES_LINES_OPTIONS = [
  ...NAVAL_LOSSES_OPTIONS
    .filter(field => ['view', 'kind', 'attribution', 'period', 'endLabels', 'yMin', 'yMax', 'log'].includes(field.key))
    .map(field => ({ ...field, visibleWhen: undefined })),
  ...LINES_VIEW_OPTIONS,
];
const NUCLEAR_ARSENAL_LINES_OPTIONS = [
  ...NUCLEAR_ARSENAL_OPTIONS
    .filter(field => ['view', 'endLabels', 'yMin', 'yMax', 'log'].includes(field.key))
    .map(field => ({ ...field, visibleWhen: undefined })),
  ...LINES_VIEW_OPTIONS,
];
const OCCUPATION_RESERVE_BREAKDOWN_OPTIONS = OCCUPATION_RESERVE_OPTIONS.filter(field => ['show', 'holder', 'yMin', 'yMax'].includes(field.key));
const RAID_TALLY_LINES_OPTIONS = [
  ...RAID_TALLY_OPTIONS.filter(field => ['view', 'metric', 'endLabels', 'yMin', 'yMax', 'log'].includes(field.key)),
  ...LINES_VIEW_OPTIONS,
];
const WORLD_TENSION_VIEW_FIELD = WORLD_TENSION_OPTIONS.filter(field => field.key === 'view');
const WORLD_TENSION_TENSION_OPTIONS = [...WORLD_TENSION_VIEW_FIELD, ...TENSION_CHART_OPTIONS];

function timeChartSpec<TModule>(
  kind: string,
  view: string,
  load: () => Promise<TModule>,
  pick: (mod: TModule) => { readonly projection: TimeChartProjection<any>; readonly declaration: TimeChartWidgetDeclaration<any, any> },
): () => Promise<FamilyViewModule> {
  return () => load().then(mod => {
    const { projection, declaration } = pick(mod);
    return { family: 'time-chart', kind, view, projection, declaration };
  });
}

export const TIME_CHART_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'air-force-size',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    options: LINES_VIEW_OPTIONS,
    viewOption: 'view',
    loadSpec: timeChartSpec(
      'air-force-size',
      'lines',
      () => import('../air-force-size/airForceLinesProjection.ts'),
      mod => ({ projection: mod.airForceLinesProjection, declaration: mod.airForceLinesDeclaration }),
    ),
  },
  {
    kind: 'air-kills-losses',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    viewOption: 'view',
    options: AIR_KILLS_LOSSES_LINES_OPTIONS,
    loadSpec: timeChartSpec(
      'air-kills-losses',
      'lines',
      () => import('../air-kills-losses/airKillsLossesLinesProjection.ts'),
      mod => ({ projection: mod.airKillsLossesLinesProjection, declaration: mod.airKillsLossesLinesDeclaration }),
    ),
  },
  {
    kind: 'apm-graph',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'apm-graph',
      DEFAULT_VIEW,
      () => import('../apm-graph/apmGraphProjection.ts'),
      mod => ({ projection: mod.apmGraphProjection, declaration: mod.apmGraphDeclaration }),
    ),
  },
  {
    kind: 'army-losses-by-month',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'army-losses-by-month',
      DEFAULT_VIEW,
      () => import('../army-losses-by-month/armyLossesProjection.ts'),
      mod => ({ projection: mod.armyLossesProjection, declaration: mod.armyLossesDeclaration }),
    ),
  },
  {
    kind: 'army-size',
    view: 'breakdown',
    label: 'Army breakdown',
    family: 'time-chart',
    viewOption: 'view',
    options: ARMY_SIZE_BREAKDOWN_OPTIONS,
    loadSpec: timeChartSpec(
      'army-size',
      'breakdown',
      () => import('../army-size/armySizeBreakdownProjection.ts'),
      mod => ({ projection: mod.armySizeBreakdownProjection, declaration: mod.armySizeBreakdownDeclaration }),
    ),
  },
  {
    kind: 'army-size',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    options: LINES_VIEW_OPTIONS,
    viewOption: 'view',
    loadSpec: timeChartSpec(
      'army-size',
      'lines',
      () => import('../army-size/armyLinesProjection.ts'),
      mod => ({ projection: mod.armyLinesProjection, declaration: mod.armyLinesDeclaration }),
    ),
  },
  {
    kind: 'command-power',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'command-power',
      DEFAULT_VIEW,
      () => import('../command-power/commandPowerProjection.ts'),
      mod => ({ projection: mod.commandPowerProjection, declaration: mod.commandPowerDeclaration }),
    ),
  },
  {
    kind: 'dockyard-spending',
    view: 'months',
    label: 'By month',
    family: 'time-chart',
    viewOption: 'view',
    options: DOCKYARD_MONTHS_OPTIONS,
    loadSpec: timeChartSpec(
      'dockyard-spending',
      'months',
      () => import('../dockyard-spending/dockyardMonthsProjection.ts'),
      mod => ({ projection: mod.dockyardMonthsProjection, declaration: mod.dockyardMonthsDeclaration }),
    ),
  },
  {
    kind: 'equipment-stockpile',
    view: 'time',
    label: 'Over time',
    family: 'time-chart',
    viewOption: 'view',
    options: STOCKPILE_TIME_OPTIONS,
    loadSpec: timeChartSpec(
      'equipment-stockpile',
      'time',
      () => import('../equipment-stockpile/stockpileTimeProjection.ts'),
      mod => ({ projection: mod.stockpileTimeProjection, declaration: mod.stockpileTimeDeclaration }),
    ),
  },
  {
    kind: 'experience',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'experience',
      DEFAULT_VIEW,
      () => import('../experience/experienceProjection.ts'),
      mod => ({ projection: mod.experienceProjection, declaration: mod.experienceDeclaration }),
    ),
  },
  {
    kind: 'factories',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    options: LINES_VIEW_OPTIONS,
    viewOption: 'view',
    loadSpec: timeChartSpec(
      'factories',
      'lines',
      () => import('../factories/factoriesLinesProjection.ts'),
      mod => ({ projection: mod.factoriesLinesProjection, declaration: mod.factoriesLinesDeclaration }),
    ),
  },
  {
    kind: 'faction-strength',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    viewOption: 'view',
    options: FACTION_STRENGTH_LINES_OPTIONS,
    loadSpec: timeChartSpec(
      'faction-strength',
      'lines',
      () => import('../faction-strength/factionStrengthLinesProjection.ts'),
      mod => ({ projection: mod.factionStrengthLinesProjection, declaration: mod.factionStrengthLinesDeclaration }),
    ),
  },
  {
    kind: 'fuel',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'fuel',
      DEFAULT_VIEW,
      () => import('../fuel/fuelProjection.ts'),
      mod => ({ projection: mod.fuelProjection, declaration: mod.fuelDeclaration }),
    ),
  },
  {
    kind: 'game-score',
    view: 'lines',
    label: 'Lines over time',
    family: 'time-chart',
    viewOption: 'view',
    options: GAME_SCORE_LINES_OPTIONS,
    loadSpec: timeChartSpec(
      'game-score',
      'lines',
      () => import('../game-score/gameScoreLinesProjection.ts'),
      mod => ({ projection: mod.gameScoreLinesProjection, declaration: mod.gameScoreLinesDeclaration }),
    ),
  },
  {
    kind: 'military-factory-spending',
    view: 'months',
    label: 'By month',
    family: 'time-chart',
    viewOption: 'view',
    options: FACTORY_MONTHS_OPTIONS,
    loadSpec: timeChartSpec(
      'military-factory-spending',
      'months',
      () => import('../military-factory-spending/factoryMonthsProjection.ts'),
      mod => ({ projection: mod.factoryMonthsProjection, declaration: mod.factoryMonthsDeclaration }),
    ),
  },
  {
    kind: 'resource-balance',
    view: 'time',
    label: 'Over time',
    family: 'time-chart',
    viewOption: 'view',
    options: RESOURCE_TIME_OPTIONS,
    loadSpec: timeChartSpec(
      'resource-balance',
      'time',
      () => import('../resource-balance/resourceTimeProjection.ts'),
      mod => ({ projection: mod.resourceTimeProjection, declaration: mod.resourceTimeDeclaration }),
    ),
  },
  {
    kind: 'naval-losses',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    viewOption: 'view',
    options: NAVAL_LOSSES_LINES_OPTIONS,
    loadSpec: timeChartSpec(
      'naval-losses',
      'lines',
      () => import('../naval-losses/navalLossesLinesProjection.ts'),
      mod => ({ projection: mod.navalLossesLinesProjection, declaration: mod.navalLossesLinesDeclaration }),
    ),
  },
  {
    kind: 'navy-size',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    options: LINES_VIEW_OPTIONS,
    viewOption: 'view',
    loadSpec: timeChartSpec(
      'navy-size',
      'lines',
      () => import('../navy-size/navyLinesProjection.ts'),
      mod => ({ projection: mod.navyLinesProjection, declaration: mod.navyLinesDeclaration }),
    ),
  },
  {
    kind: 'nuclear-arsenal',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    viewOption: 'view',
    options: NUCLEAR_ARSENAL_LINES_OPTIONS,
    loadSpec: timeChartSpec(
      'nuclear-arsenal',
      'lines',
      () => import('../nuclear-arsenal/nuclearArsenalLinesProjection.ts'),
      mod => ({ projection: mod.nuclearArsenalLinesProjection, declaration: mod.nuclearArsenalLinesDeclaration }),
    ),
  },
  {
    kind: 'occupation-reserve-manpower',
    view: 'breakdown',
    label: 'Holder breakdown',
    family: 'time-chart',
    viewOption: 'show',
    options: OCCUPATION_RESERVE_BREAKDOWN_OPTIONS,
    loadSpec: timeChartSpec(
      'occupation-reserve-manpower',
      'breakdown',
      () => import('../occupation-reserve-manpower/occupationReserveBreakdownProjection.ts'),
      mod => ({ projection: mod.occupationReserveBreakdownProjection, declaration: mod.occupationReserveBreakdownDeclaration }),
    ),
  },
  {
    kind: 'occupation-trend',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    options: LINES_VIEW_OPTIONS,
    viewOption: 'view',
    loadSpec: timeChartSpec(
      'occupation-trend',
      'lines',
      () => import('../occupation-trend/occupationLinesProjection.ts'),
      mod => ({ projection: mod.occupationLinesProjection, declaration: mod.occupationLinesDeclaration }),
    ),
  },
  {
    kind: 'political-power',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'political-power',
      DEFAULT_VIEW,
      () => import('../political-power/politicalPowerProjection.ts'),
      mod => ({ projection: mod.politicalPowerProjection, declaration: mod.politicalPowerDeclaration }),
    ),
  },
  {
    kind: 'raid-tally',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    viewOption: 'view',
    options: RAID_TALLY_LINES_OPTIONS,
    loadSpec: timeChartSpec(
      'raid-tally',
      'lines',
      () => import('../raid-tally/raidTallyLinesProjection.ts'),
      mod => ({ projection: mod.raidTallyLinesProjection, declaration: mod.raidTallyLinesDeclaration }),
    ),
  },
  {
    kind: 'ruling-party',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'ruling-party',
      DEFAULT_VIEW,
      () => import('../ruling-party/rulingPartyProjection.ts'),
      mod => ({ projection: mod.rulingPartyProjection, declaration: mod.rulingPartyDeclaration }),
    ),
  },
  {
    kind: 'balance-of-power',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'balance-of-power',
      DEFAULT_VIEW,
      () => import('../balance-of-power/balanceOfPowerProjection.ts'),
      mod => ({ projection: mod.balanceOfPowerProjection, declaration: mod.balanceOfPowerDeclaration }),
    ),
  },
  {
    kind: 'stability',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'stability',
      DEFAULT_VIEW,
      () => import('../stability/stabilityProjection.ts'),
      mod => ({ projection: mod.stabilityProjection, declaration: mod.stabilityDeclaration }),
    ),
  },
  {
    kind: 'supply-health',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'supply-health',
      DEFAULT_VIEW,
      () => import('../supply-health/supplyHealthProjection.ts'),
      mod => ({ projection: mod.supplyHealthProjection, declaration: mod.supplyHealthDeclaration }),
    ),
  },
  {
    kind: 'world-tension',
    view: 'tension',
    label: 'World tension',
    family: 'time-chart',
    viewOption: 'view',
    options: WORLD_TENSION_TENSION_OPTIONS,
    loadSpec: timeChartSpec(
      'world-tension',
      'tension',
      () => import('../world-tension/worldTensionProjection.ts'),
      mod => ({ projection: mod.worldTensionProjection, declaration: mod.worldTensionDeclaration }),
    ),
  },
  {
    kind: 'world-tension',
    view: 'lines',
    label: 'Contributions over time',
    family: 'time-chart',
    viewOption: 'view',
    options: WORLD_TENSION_VIEW_FIELD,
    loadSpec: timeChartSpec(
      'world-tension',
      'lines',
      () => import('../world-tension/worldTensionLinesProjection.ts'),
      mod => ({ projection: mod.worldTensionLinesProjection, declaration: mod.worldTensionLinesDeclaration }),
    ),
  },
];

export const timeChartFamilyManifest: FamilyManifestModule = {
  family: 'time-chart',
  lane: () => import('../../families/time-chart/laneView.ts').then(m => m.timeChartLane),
  views: TIME_CHART_VIEWS,
  hostAdapter: timeChartHostAdapter,
};
