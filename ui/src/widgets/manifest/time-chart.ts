import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry, type FamilyViewModule } from '../../families/manifest.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import { timeChartLaneView } from '../../families/time-chart/laneView.ts';
import { OPTIONS as WORLD_TENSION_OPTIONS, TENSION_CHART_OPTIONS } from '../world-tension/definition.ts';

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
    viewOption: 'view',
    loadSpec: timeChartSpec(
      'air-force-size',
      'lines',
      () => import('../air-force-size/airForceLinesProjection.ts'),
      mod => ({ projection: mod.airForceLinesProjection, declaration: mod.airForceLinesDeclaration }),
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
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
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
    viewOption: 'view',
    loadSpec: timeChartSpec(
      'factories',
      'lines',
      () => import('../factories/factoriesLinesProjection.ts'),
      mod => ({ projection: mod.factoriesLinesProjection, declaration: mod.factoriesLinesDeclaration }),
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
    kind: 'navy-size',
    view: 'lines',
    label: 'Lines',
    family: 'time-chart',
    viewOption: 'view',
    loadSpec: timeChartSpec(
      'navy-size',
      'lines',
      () => import('../navy-size/navyLinesProjection.ts'),
      mod => ({ projection: mod.navyLinesProjection, declaration: mod.navyLinesDeclaration }),
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
  laneView: timeChartLaneView,
  views: TIME_CHART_VIEWS,
  hostAdapter: timeChartHostAdapter,
};
