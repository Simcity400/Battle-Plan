import { DEFAULT_VIEW, type FamilyViewEntry, type FamilyViewModule } from '../families/manifest.ts';
import { bindFamilyManifest } from '../families/registry.ts';
import type { AnyTableWidgetDeclaration } from '../families/table/declaration.ts';
import type { TableProjection } from '../families/table/model.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../families/time-chart/declaration.ts';
import type { BreakdownProjection } from '../families/breakdown/model.ts';
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

function breakdownViews<TModule>(
  kind: string,
  load: () => Promise<TModule>,
  pick: (mod: TModule) => BreakdownProjection<any>,
): readonly FamilyViewEntry[] {
  const views = [
    { view: 'parts', label: 'Parts' },
    { view: 'ring', label: 'Ring' },
    { view: 'mosaic', label: 'Mosaic' },
  ] as const;
  return views.map(({ view, label }) => ({
    kind,
    view,
    label,
    family: 'breakdown',
    viewOption: 'view',
    loadSpec: () => load().then(mod => {
      const projection = { ...pick(mod), family: 'breakdown' as const };
      return { family: 'breakdown', kind, view, projection };
    }),
  }));
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
  ...breakdownViews(
    'political-power-spending-breakdown',
    () => import('./political-power-spending-breakdown/ppSpendingProjection.ts'),
    mod => mod.ppSpendingProjection,
  ),
  ...breakdownViews(
    'territory-breakdown',
    () => import('./territory-breakdown/territoryBreakdownProjection.ts'),
    mod => mod.territoryBreakdownProjection,
  ),
  ...breakdownViews(
    'equipment-losses',
    () => import('./equipment-losses/equipmentLossesProjection.ts'),
    mod => mod.equipmentLossesProjection,
  ),
  ...breakdownViews(
    'production-spending',
    () => import('./production-spending/productionSpendingProjection.ts'),
    mod => mod.productionSpendingProjection,
  ),
  ...breakdownViews(
    'dockyard-spending',
    () => import('./dockyard-spending/purposeSpendingProjection.ts'),
    mod => mod.dockyardSpendingProjection,
  ),
  ...breakdownViews(
    'military-factory-spending',
    () => import('./dockyard-spending/purposeSpendingProjection.ts'),
    mod => mod.militaryFactorySpendingProjection,
  ),
  ...breakdownViews(
    'bombing-damage-breakdown',
    () => import('./bombing-damage-breakdown/bombingBreakdownProjection.ts'),
    mod => mod.bombingBreakdownProjection,
  ),
  ...breakdownViews(
    'construction-spending',
    () => import('./construction-spending/constructionSpendingProjection.ts'),
    mod => mod.constructionSpendingProjection,
  ),
  ...breakdownViews(
    'casualties-breakdown',
    () => import('./casualties-breakdown/casualtiesBreakdownProjection.ts'),
    mod => mod.casualtiesBreakdownProjection,
  ),
  {
    kind: 'territory',
    view: 'share',
    label: 'Share',
    family: 'breakdown',
    viewOption: 'view',
    loadSpec: () => import('./territory/territoryShareProjection.ts').then(mod => ({
      family: 'breakdown',
      kind: 'territory',
      view: 'share',
      projection: { ...mod.territoryShareProjection, family: 'breakdown' as const },
    })),
  },
];

bindFamilyManifest(FAMILY_VIEWS);
