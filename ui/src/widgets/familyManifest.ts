import { DEFAULT_VIEW, type FamilyViewEntry, type FamilyViewModule } from '../families/manifest.ts';
import { bindFamilyManifest } from '../families/registry.ts';
import type { AnyTableWidgetDeclaration } from '../families/table/declaration.ts';
import type { TableProjection } from '../families/table/model.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../families/time-chart/declaration.ts';
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
    kind: 'command-power',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: timeChartSpec(
      'command-power',
      DEFAULT_VIEW,
      () => import('./command-power/commandPowerProjection.ts'),
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
      () => import('./stability/stabilityProjection.ts'),
      mod => ({ projection: mod.stabilityProjection, declaration: mod.stabilityDeclaration }),
    ),
  },
];

bindFamilyManifest(FAMILY_VIEWS);
