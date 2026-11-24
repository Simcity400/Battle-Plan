import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry, type FamilyViewModule } from '../../families/manifest.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import { timeChartLaneView } from '../../families/time-chart/laneView.ts';

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
];

export const timeChartFamilyManifest: FamilyManifestModule = {
  family: 'time-chart',
  laneView: timeChartLaneView,
  views: TIME_CHART_VIEWS,
  hostAdapter: timeChartHostAdapter,
};
