import { tableHostAdapter } from '../../families/table/hostAdapter.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry, type FamilyViewModule } from '../../families/manifest.ts';
import type { AnyTableWidgetDeclaration } from '../../families/table/declaration.ts';
import type { TableProjection } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { OPTIONS as GAME_SCORE_OPTIONS } from '../game-score/definition.ts';
import * as DIVISION_EQUIPMENT from '../division-equipment/definition.ts';
import { MARKET_TABS, VIEWS, marketTableOptions } from '../international-market/definition.ts';
import { EVENTS_TABLE_OPTIONS, OPTIONS as WORLD_TENSION_OPTIONS } from '../world-tension/definition.ts';

const WORLD_TENSION_EVENTS_SCOPED_OPTIONS = [
  ...WORLD_TENSION_OPTIONS.filter(field => field.key === 'view'),
  ...EVENTS_TABLE_OPTIONS,
];

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

const DIVISION_EQUIPMENT_SHARED = DIVISION_EQUIPMENT.OPTIONS.filter(field => field.key === 'country' || field.key === 'view' || field.key === 'countFor');
const GAME_SCORE_TABLE_OPTION_KEYS = new Set(['view', 'factions', 'topMeasure', 'rankMarkers', TABLE_SETTINGS_KEY]);
const GAME_SCORE_TABLE_OPTIONS = GAME_SCORE_OPTIONS.filter(field => GAME_SCORE_TABLE_OPTION_KEYS.has(field.key));

export const TABLE_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'war-contribution',
    view: 'sides',
    label: 'Sides',
    family: 'table',
    viewOption: 'view',
    loadSpec: tableSpec(
      'war-contribution',
      'sides',
      () => import('../war-contribution/sidesProjection.ts'),
      mod => ({ projection: mod.warContributionSidesProjection, declaration: mod.warContributionSidesDeclaration }),
    ),
  },
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
  ...(['political-power-spending', 'command-power-spending'] as const).map((kind): FamilyViewEntry => ({
    kind,
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      kind,
      DEFAULT_VIEW,
      () => import('../political-power-spending/powerSpendingProjection.ts'),
      mod => (kind === 'political-power-spending'
        ? { projection: mod.politicalSpendingProjection, declaration: mod.politicalSpendingDeclaration }
        : { projection: mod.commandSpendingProjection, declaration: mod.commandSpendingDeclaration }),
    ),
  })),
  {
    kind: 'production-lines',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'production-lines',
      DEFAULT_VIEW,
      () => import('../production-lines/productionLinesProjection.ts'),
      mod => ({ projection: mod.productionLinesProjection, declaration: mod.productionLinesDeclaration }),
    ),
  },
  ...MARKET_TABS.map((view): FamilyViewEntry => ({
    kind: 'international-market',
    view,
    label: VIEWS.find(choice => choice.value === view)!.label,
    family: 'table',
    viewOption: 'view',
    options: marketTableOptions(view),
    loadSpec: tableSpec(
      'international-market',
      view,
      () => import('../international-market/internationalMarketProjection.ts'),
      mod => ({ projection: mod.internationalMarketProjection, declaration: mod.MARKET_DECLARATIONS[view] }),
    ),
  })),
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
  {
    kind: 'division-equipment',
    view: 'equipment',
    label: 'By equipment',
    family: 'table',
    viewOption: 'view',
    options: [...DIVISION_EQUIPMENT_SHARED, DIVISION_EQUIPMENT.TYPE_TABLE_OPTION],
    loadSpec: tableSpec(
      'division-equipment',
      'equipment',
      () => import('../division-equipment/divisionEquipmentProjection.ts'),
      mod => ({ projection: mod.equipmentTypeProjection, declaration: mod.equipmentTypeDeclaration }),
    ),
  },
  {
    kind: 'division-equipment',
    view: 'template',
    label: 'By template',
    family: 'table',
    viewOption: 'view',
    options: [...DIVISION_EQUIPMENT_SHARED, DIVISION_EQUIPMENT.TEMPLATE_TABLE_OPTION],
    loadSpec: tableSpec(
      'division-equipment',
      'template',
      () => import('../division-equipment/divisionEquipmentProjection.ts'),
      mod => ({ projection: mod.equipmentTemplateProjection, declaration: mod.equipmentTemplateDeclaration }),
    ),
  },
  {
    kind: 'aces',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'aces',
      DEFAULT_VIEW,
      () => import('../aces/acesProjection.ts'),
      mod => ({ projection: mod.acesProjection, declaration: mod.acesDeclaration }),
    ),
  },
  {
    kind: 'faction-goals',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'faction-goals',
      DEFAULT_VIEW,
      () => import('../faction-goals/factionGoalsProjection.ts'),
      mod => ({ projection: mod.factionGoalsProjection, declaration: mod.factionGoalsDeclaration }),
    ),
  },
  {
    kind: 'generals',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'generals',
      DEFAULT_VIEW,
      () => import('../generals/generalsProjection.ts'),
      mod => ({ projection: mod.generalsProjection, declaration: mod.generalsDeclaration }),
    ),
  },
  {
    kind: 'most-decorated-divisions',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'most-decorated-divisions',
      DEFAULT_VIEW,
      () => import('../most-decorated-divisions/mostDecoratedProjection.ts'),
      mod => ({ projection: mod.mostDecoratedProjection, declaration: mod.mostDecoratedDeclaration }),
    ),
  },
  {
    kind: 'trade-flows',
    view: 'table',
    label: 'Table',
    family: 'table',
    viewOption: 'view',
    loadSpec: () => import('../trade-flows/tradeFlowsProjection.ts').then(mod => ({
      family: 'table', kind: 'trade-flows', view: 'table',
      projection: mod.tradeTableProjection, declaration: mod.tradeTableDeclaration,
    })),
  },
  {
    kind: 'world-tension',
    view: 'events',
    label: 'Threat events',
    family: 'table',
    viewOption: 'view',
    options: WORLD_TENSION_EVENTS_SCOPED_OPTIONS,
    loadSpec: tableSpec(
      'world-tension',
      'events',
      () => import('../world-tension/worldTensionEventsProjection.ts'),
      mod => ({ projection: mod.worldTensionEventsProjection, declaration: mod.worldTensionEventsDeclaration }),
    ),
  },
  {
    kind: 'market-ledger',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'market-ledger',
      DEFAULT_VIEW,
      () => import('../market-ledger/marketLedgerProjection.ts'),
      mod => ({ projection: mod.marketLedgerProjection, declaration: mod.marketLedgerDeclaration }),
    ),
  },
  {
    kind: 'decisions',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'decisions',
      DEFAULT_VIEW,
      () => import('../decisions/decisionsProjection.ts'),
      mod => ({ projection: mod.decisionsProjection, declaration: mod.decisionsDeclaration }),
    ),
  },
  {
    kind: 'event-choices',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'event-choices',
      DEFAULT_VIEW,
      () => import('../event-choices/eventChoicesProjection.ts'),
      mod => ({ projection: mod.eventChoicesProjection, declaration: mod.eventChoicesDeclaration }),
    ),
  },
  {
    kind: 'event-feed',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: tableSpec(
      'event-feed',
      DEFAULT_VIEW,
      () => import('../event-feed/eventFeedProjection.ts'),
      mod => ({ projection: mod.eventFeedProjection, declaration: mod.eventFeedDeclaration }),
    ),
  },
];

export const tableFamilyManifest: FamilyManifestModule = {
  family: 'table',
  lane: () => import('../../families/table/laneView.ts').then(m => m.tableLane),
  views: TABLE_VIEWS,
  hostAdapter: tableHostAdapter,
};
