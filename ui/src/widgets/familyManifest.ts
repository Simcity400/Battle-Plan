import { DEFAULT_VIEW, type FamilyViewEntry, type FamilyViewModule } from '../families/manifest.ts';
import { bindFamilyManifest } from '../families/registry.ts';
import type { AnyBarsWidgetDeclaration } from '../families/bars/declaration.ts';
import type { BarsProjection } from '../families/bars/model.ts';
import type { AnyHeatMatrixWidgetDeclaration } from '../families/heat/matrixDeclaration.ts';
import type { HeatMatrixProjection } from '../families/heat/matrixModel.ts';
import type { AnyTableWidgetDeclaration } from '../families/table/declaration.ts';
import type { TableProjection } from '../families/table/model.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../families/time-chart/declaration.ts';
import { OPTIONS as CASUALTIES_BETWEEN_OPTIONS } from './casualties-between/definition.ts';
import { OPTIONS as GAME_SCORE_OPTIONS } from './game-score/definition.ts';

function tableSpec<TModule>(
  kind: string,
  view: string,
  load: () => Promise<TModule>,
  pick: (mod: TModule) => { readonly projection: TableProjection<any>; readonly declaration: AnyTableWidgetDeclaration },
): () => Promise<FamilyViewModule> {
  return () => load().then(mod => {
    const { projection, declaration } = pick(mod);
    return { family: 'table', kind, view, projection, declaration };
  });
}

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

function heatMatrixSpec<TModule>(
  kind: string,
  view: string,
  load: () => Promise<TModule>,
  pick: (mod: TModule) => { readonly projection: HeatMatrixProjection<any>; readonly declaration: AnyHeatMatrixWidgetDeclaration },
): () => Promise<FamilyViewModule> {
  return () => load().then(mod => {
    const { projection, declaration } = pick(mod);
    return { family: 'heat-matrix', kind, view, projection, declaration };
  });
}

function barsSpec<TModule>(
  kind: string,
  view: string,
  load: () => Promise<TModule>,
  pick: (mod: TModule) => { readonly projection: BarsProjection<any>; readonly declaration: AnyBarsWidgetDeclaration },
): () => Promise<FamilyViewModule> {
  return () => load().then(mod => {
    const { projection, declaration } = pick(mod);
    return { family: 'bars', kind, view, projection, declaration };
  });
}

const GAME_SCORE_TABLE_OPTION_KEYS = new Set(['view', 'factions', 'topMeasure', 'cellBars', 'rankMarkers', 'density', 'narrowTable', 'tableSort']);
const GAME_SCORE_TABLE_OPTIONS = GAME_SCORE_OPTIONS.filter(field => GAME_SCORE_TABLE_OPTION_KEYS.has(field.key));

export const FAMILY_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'campaign-clock',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'campaign-clock',
      DEFAULT_VIEW,
      () => import('./campaign-clock/clockProjection.ts'),
      mod => ({ projection: mod.campaignClockProjection, declaration: mod.campaignClockDeclaration }),
    ),
  },
  {
    kind: 'occupied-states',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'occupied-states',
      DEFAULT_VIEW,
      () => import('./occupied-states/occupiedProjection.ts'),
      mod => ({ projection: mod.occupiedStatesProjection, declaration: mod.occupiedStatesDeclaration }),
    ),
  },
  {
    kind: 'scoreboard',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'scoreboard',
      DEFAULT_VIEW,
      () => import('./scoreboard/scoreboardProjection.ts'),
      mod => ({ projection: mod.scoreboardProjection, declaration: mod.scoreboardDeclaration }),
    ),
  },
  {
    kind: 'apm-table',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'apm-table',
      DEFAULT_VIEW,
      () => import('./player-apm/apmProjection.ts'),
      mod => ({ projection: mod.apmTableProjection, declaration: mod.apmTableDeclaration }),
    ),
  },
  {
    kind: 'division-template',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'division-template',
      DEFAULT_VIEW,
      () => import('./division-template/divisionTemplateProjection.ts'),
      mod => ({ projection: mod.divisionTemplateProjection, declaration: mod.divisionTemplateDeclaration }),
    ),
  },
  {
    kind: 'game-score',
    view: 'table',
    label: 'Table',
    family: 'table',
    viewOption: 'view',
    options: GAME_SCORE_TABLE_OPTIONS,
    loadSpec: tableSpec(
      'game-score',
      'table',
      () => import('./game-score/gameScoreProjection.ts'),
      mod => ({ projection: mod.gameScoreProjection, declaration: mod.gameScoreDeclaration }),
    ),
  },
  {
    kind: 'political-power',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'political-power',
      DEFAULT_VIEW,
      () => import('./political-power/politicalPowerProjection.ts'),
      mod => ({ projection: mod.politicalPowerProjection, declaration: mod.politicalPowerDeclaration }),
    ),
  },
  {
    kind: 'who-spies-on-whom',
    view: 'matrix',
    label: 'Matrix',
    family: 'heat-matrix',
    loadSpec: heatMatrixSpec(
      'who-spies-on-whom',
      'matrix',
      () => import('./who-spies-on-whom/whoSpiesProjection.ts'),
      mod => ({ projection: mod.operativeMatrixProjection, declaration: mod.operativeMatrixDeclaration }),
    ),
  },
  {
    kind: 'who-spies-on-whom',
    view: 'pairs',
    label: 'Top pairs',
    family: 'bars',
    loadSpec: barsSpec(
      'who-spies-on-whom',
      'pairs',
      () => import('./who-spies-on-whom/whoSpiesProjection.ts'),
      mod => ({ projection: mod.operativePairsProjection, declaration: mod.operativePairsDeclaration }),
    ),
  },
  {
    kind: 'casualties-between',
    view: 'matrix',
    label: 'Matrix',
    family: 'heat-matrix',
    options: CASUALTIES_BETWEEN_OPTIONS,
    loadSpec: heatMatrixSpec(
      'casualties-between',
      'matrix',
      () => import('./casualties-between/casualtiesBetweenProjection.ts'),
      mod => ({ projection: mod.casualtyMatrixProjection, declaration: mod.casualtyMatrixDeclaration }),
    ),
  },
  {
    kind: 'casualties-between',
    view: 'opponents',
    label: 'By opponent',
    family: 'bars',
    loadSpec: barsSpec(
      'casualties-between',
      'opponents',
      () => import('./casualties-between/casualtiesBetweenProjection.ts'),
      mod => ({ projection: mod.casualtyOpponentsProjection, declaration: mod.casualtyOpponentsDeclaration }),
    ),
  },
];

bindFamilyManifest(FAMILY_VIEWS);
