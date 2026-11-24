import { DEFAULT_VIEW, type FamilyViewEntry, type FamilyViewModule } from '../families/manifest.ts';
import { bindFamilyManifest } from '../families/registry.ts';
import type { AnyTableWidgetDeclaration } from '../families/table/declaration.ts';
import type { TableProjection } from '../families/table/model.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../families/time-chart/declaration.ts';
import type { AnyTimelineWidgetDeclaration } from '../families/timeline/declaration.ts';
import type { TimelineProjection } from '../families/timeline/model.ts';
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

function timelineSpec<TModule>(
  kind: string,
  view: string,
  load: () => Promise<TModule>,
  pick: (mod: TModule) => { readonly projection: TimelineProjection<any>; readonly declaration?: AnyTimelineWidgetDeclaration },
): () => Promise<FamilyViewModule> {
  return () => load().then(mod => {
    const { projection, declaration } = pick(mod);
    return { family: 'timeline', kind, view, projection, ...(declaration ? { declaration } : {}) };
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
    kind: 'idea-timeline',
    view: DEFAULT_VIEW,
    family: 'timeline',
    loadSpec: timelineSpec(
      'idea-timeline',
      DEFAULT_VIEW,
      () => import('./idea-timeline/ideaTimelineProjection.ts'),
      mod => ({ projection: mod.ideaTimelineProjection, declaration: mod.ideaTimelineDeclaration }),
    ),
  },
  {
    kind: 'spirit-history',
    view: DEFAULT_VIEW,
    family: 'timeline',
    loadSpec: timelineSpec(
      'spirit-history',
      DEFAULT_VIEW,
      () => import('./spirit-history/spiritHistoryProjection.ts'),
      mod => ({ projection: mod.spiritHistoryProjection, declaration: mod.spiritHistoryDeclaration }),
    ),
  },
  {
    kind: 'civil-wars',
    view: DEFAULT_VIEW,
    family: 'timeline',
    loadSpec: timelineSpec(
      'civil-wars',
      DEFAULT_VIEW,
      () => import('./civil-wars/civilWarsProjection.ts'),
      mod => ({ projection: mod.civilWarsProjection, declaration: mod.civilWarsDeclaration }),
    ),
  },
  {
    kind: 'factions',
    view: DEFAULT_VIEW,
    family: 'timeline',
    loadSpec: timelineSpec(
      'factions',
      DEFAULT_VIEW,
      () => import('./factions/factionsProjection.ts'),
      mod => ({ projection: mod.factionsProjection, declaration: mod.factionsDeclaration }),
    ),
  },
  {
    kind: 'subjects',
    view: DEFAULT_VIEW,
    family: 'timeline',
    loadSpec: timelineSpec(
      'subjects',
      DEFAULT_VIEW,
      () => import('./subjects/subjectsProjection.ts'),
      mod => ({ projection: mod.subjectsProjection, declaration: mod.subjectsDeclaration }),
    ),
  },
  {
    kind: 'diplomatic-pacts',
    view: DEFAULT_VIEW,
    family: 'timeline',
    loadSpec: timelineSpec(
      'diplomatic-pacts',
      DEFAULT_VIEW,
      () => import('./diplomatic-pacts/diplomaticPactsProjection.ts'),
      mod => ({ projection: mod.diplomaticPactsProjection, declaration: mod.diplomaticPactsDeclaration }),
    ),
  },
  {
    kind: 'faction-upgrades',
    view: DEFAULT_VIEW,
    family: 'timeline',
    loadSpec: timelineSpec(
      'faction-upgrades',
      DEFAULT_VIEW,
      () => import('./faction-upgrades/factionUpgradesProjection.ts'),
      mod => ({ projection: mod.factionUpgradesProjection }),
    ),
  },
  {
    kind: 'autonomy',
    view: 'meters',
    label: 'Meters',
    family: 'timeline',
    viewOption: 'view',
    loadSpec: timelineSpec(
      'autonomy',
      'meters',
      () => import('./autonomy/autonomyProjection.ts'),
      mod => ({ projection: mod.autonomyProjection }),
    ),
  },
  {
    kind: 'autonomy',
    view: 'history',
    label: 'Over time',
    family: 'timeline',
    viewOption: 'view',
    loadSpec: timelineSpec(
      'autonomy',
      'history',
      () => import('./autonomy/autonomyProjection.ts'),
      mod => ({ projection: mod.autonomyProjection }),
    ),
  },
  {
    kind: 'wars',
    view: 'gantt',
    label: 'Wars',
    family: 'timeline',
    viewOption: 'view',
    loadSpec: timelineSpec(
      'wars',
      'gantt',
      () => import('./wars/warsProjection.ts'),
      mod => ({ projection: mod.warsProjection }),
    ),
  },
  {
    kind: 'wars',
    view: 'countries',
    label: 'Countries',
    family: 'timeline',
    viewOption: 'view',
    loadSpec: timelineSpec(
      'wars',
      'countries',
      () => import('./wars/warsProjection.ts'),
      mod => ({ projection: mod.warsProjection }),
    ),
  },
  {
    kind: 'general-career',
    view: 'timeline',
    label: 'Timeline',
    family: 'timeline',
    viewOption: 'view',
    loadSpec: timelineSpec(
      'general-career',
      'timeline',
      () => import('./general-career/generalCareerProjection.ts'),
      mod => ({ projection: mod.generalCareerProjection }),
    ),
  },
  {
    kind: 'general-career',
    view: 'dossier',
    label: 'Dossier',
    family: 'timeline',
    viewOption: 'view',
    loadSpec: timelineSpec(
      'general-career',
      'dossier',
      () => import('./general-career/generalCareerProjection.ts'),
      mod => ({ projection: mod.generalCareerProjection }),
    ),
  },
  {
    kind: 'general-career',
    view: 'record',
    label: 'Record',
    family: 'timeline',
    viewOption: 'view',
    loadSpec: timelineSpec(
      'general-career',
      'record',
      () => import('./general-career/generalCareerProjection.ts'),
      mod => ({ projection: mod.generalCareerProjection }),
    ),
  },
];

bindFamilyManifest(FAMILY_VIEWS);
