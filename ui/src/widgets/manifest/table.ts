import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry, type FamilyViewModule } from '../../families/manifest.ts';
import type { AnyTableWidgetDeclaration } from '../../families/table/declaration.ts';
import { familyTableLaneView } from '../../families/table/laneView.ts';
import type { TableProjection } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { OPTIONS as GAME_SCORE_OPTIONS } from '../game-score/definition.ts';

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

const GAME_SCORE_TABLE_OPTION_KEYS = new Set(['view', 'factions', 'topMeasure', 'rankMarkers', TABLE_SETTINGS_KEY]);
const GAME_SCORE_TABLE_OPTIONS = GAME_SCORE_OPTIONS.filter(field => GAME_SCORE_TABLE_OPTION_KEYS.has(field.key));

export const TABLE_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'campaign-clock',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'campaign-clock',
      DEFAULT_VIEW,
      () => import('../campaign-clock/clockProjection.ts'),
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
      () => import('../occupied-states/occupiedProjection.ts'),
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
      () => import('../scoreboard/scoreboardProjection.ts'),
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
      () => import('../player-apm/apmProjection.ts'),
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
      () => import('../division-template/divisionTemplateProjection.ts'),
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
      () => import('../game-score/gameScoreProjection.ts'),
      mod => ({ projection: mod.gameScoreProjection, declaration: mod.gameScoreDeclaration }),
    ),
  },
  {
    kind: 'governments-in-exile',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'governments-in-exile',
      DEFAULT_VIEW,
      () => import('../governments-in-exile/exilesProjection.ts'),
      mod => ({ projection: mod.exilesProjection, declaration: mod.exilesDeclaration }),
    ),
  },
  {
    kind: 'lend-lease-agreements',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'lend-lease-agreements',
      DEFAULT_VIEW,
      () => import('../lend-lease-agreements/agreementsProjection.ts'),
      mod => ({ projection: mod.agreementsProjection, declaration: mod.agreementsDeclaration }),
    ),
  },
  {
    kind: 'territory-transfers',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'territory-transfers',
      DEFAULT_VIEW,
      () => import('../territory-transfers/transfersProjection.ts'),
      mod => ({ projection: mod.transfersProjection, declaration: mod.transfersDeclaration }),
    ),
  },
  {
    kind: 'operatives-roster',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'operatives-roster',
      DEFAULT_VIEW,
      () => import('../operatives-roster/rosterProjection.ts'),
      mod => ({ projection: mod.rosterProjection, declaration: mod.rosterDeclaration }),
    ),
  },
  {
    kind: 'industrial-organisations',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'industrial-organisations',
      DEFAULT_VIEW,
      () => import('../industrial-organisations/organisationsProjection.ts'),
      mod => ({ projection: mod.organisationsProjection, declaration: mod.organisationsDeclaration }),
    ),
  },
  {
    kind: 'agency-comparison',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'agency-comparison',
      DEFAULT_VIEW,
      () => import('../agency-comparison/agencyProjection.ts'),
      mod => ({ projection: mod.agencyProjection, declaration: mod.agencyDeclaration }),
    ),
  },
  {
    kind: 'equipment-designs',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'equipment-designs',
      DEFAULT_VIEW,
      () => import('../equipment-designs/designsProjection.ts'),
      mod => ({ projection: mod.designsProjection, declaration: mod.designsDeclaration }),
    ),
  },
  {
    kind: 'next-elections',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'next-elections',
      DEFAULT_VIEW,
      () => import('../next-elections/electionsProjection.ts'),
      mod => ({ projection: mod.electionsProjection, declaration: mod.electionsDeclaration }),
    ),
  },
  {
    kind: 'air-missions',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'air-missions',
      DEFAULT_VIEW,
      () => import('../air-missions/airMissionsProjection.ts'),
      mod => ({ projection: mod.airMissionsProjection, declaration: mod.airMissionsDeclaration }),
    ),
  },
  {
    kind: 'dockyards-and-repairs',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'dockyards-and-repairs',
      DEFAULT_VIEW,
      () => import('../dockyards-and-repairs/dockyardsProjection.ts'),
      mod => ({ projection: mod.dockyardsProjection, declaration: mod.dockyardsDeclaration }),
    ),
  },
  {
    kind: 'refits',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'refits',
      DEFAULT_VIEW,
      () => import('../refits/refitsProjection.ts'),
      mod => ({ projection: mod.refitsProjection, declaration: mod.refitsDeclaration }),
    ),
  },
  {
    kind: 'training-queue',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'training-queue',
      DEFAULT_VIEW,
      () => import('../training-queue/trainingQueueProjection.ts'),
      mod => ({ projection: mod.trainingQueueProjection, declaration: mod.trainingQueueDeclaration }),
    ),
  },
];

export const tableFamilyManifest: FamilyManifestModule = {
  family: 'table',
  laneView: familyTableLaneView,
  views: TABLE_VIEWS,
};
