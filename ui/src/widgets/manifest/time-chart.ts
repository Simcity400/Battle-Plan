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
