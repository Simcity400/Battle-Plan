import type { OptionField } from '../../dashboard/options.ts';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { familyBarsLaneView } from '../../families/bars/laneView.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';
import { OPTIONS as AIR_FORCE_OPTIONS } from '../air-force-size/definition.ts';
import { OPTIONS as GAME_SCORE_OPTIONS } from '../game-score/definition.ts';
import { OPTIONS as NAVAL_LOSSES_OPTIONS } from '../naval-losses/definition.ts';
import { OPTIONS as NAVY_SIZE_OPTIONS } from '../navy-size/definition.ts';
import { OPTIONS as NUCLEAR_ARSENAL_OPTIONS } from '../nuclear-arsenal/definition.ts';
import { OPTIONS as OCCUPATION_TREND_OPTIONS } from '../occupation-trend/definition.ts';
import { OPTIONS as WORLD_TENSION_OPTIONS } from '../world-tension/definition.ts';

function raceScopedOptions(fields: readonly OptionField[], keys: ReadonlySet<string>): readonly OptionField[] {
  return fields
    .filter(field => keys.has(field.key))
    .map(field => (field.key === 'view' || field.key === 'split' ? { ...field, visibleWhen: undefined } : field));
}

const AIR_FORCE_RACE_OPTION_KEYS = new Set(['view', 'measure', 'ownership', 'split']);
const AIR_FORCE_RACE_OPTIONS = raceScopedOptions(AIR_FORCE_OPTIONS, AIR_FORCE_RACE_OPTION_KEYS);
const GAME_SCORE_RACE_OPTION_KEYS = new Set(['view', 'topMeasure', 'split']);
const GAME_SCORE_RACE_OPTIONS = GAME_SCORE_OPTIONS.filter(field => GAME_SCORE_RACE_OPTION_KEYS.has(field.key));
const NAVAL_LOSSES_RACE_OPTION_KEYS = new Set(['view', 'kind', 'attribution', 'period', 'split']);
const NAVAL_LOSSES_RACE_OPTIONS = raceScopedOptions(NAVAL_LOSSES_OPTIONS, NAVAL_LOSSES_RACE_OPTION_KEYS)
  .map((field): OptionField => (field.key === 'split' ? { ...field, visibleWhen: options => options.kind === 'both' } : field));
const NAVY_SIZE_RACE_OPTION_KEYS = new Set(['view', 'measure', 'ownership', 'grouping', 'split']);
const NAVY_SIZE_RACE_OPTIONS = raceScopedOptions(NAVY_SIZE_OPTIONS, NAVY_SIZE_RACE_OPTION_KEYS);
const NUCLEAR_ARSENAL_RACE_OPTION_KEYS = new Set(['view', 'split']);
const NUCLEAR_ARSENAL_RACE_OPTIONS = raceScopedOptions(NUCLEAR_ARSENAL_OPTIONS, NUCLEAR_ARSENAL_RACE_OPTION_KEYS);
const OCCUPATION_TREND_RACE_OPTION_KEYS = new Set(['view', 'measure', 'rankingMeasure', 'digits']);
const OCCUPATION_TREND_RACE_OPTIONS = OCCUPATION_TREND_OPTIONS.filter(field => OCCUPATION_TREND_RACE_OPTION_KEYS.has(field.key));
const WORLD_TENSION_RACE_OPTIONS = WORLD_TENSION_OPTIONS.filter(field => field.key === 'view');

export const BARS_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'war-contribution',
    view: 'measure',
    label: 'By measure',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../war-contribution/measureProjection.ts').then(m => ({
      family: 'bars', kind: 'war-contribution', view: 'measure',
      projection: m.warContributionMeasureProjection, declaration: m.warContributionMeasureDeclaration,
    })),
  },
  {
    kind: 'air-force-size',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: AIR_FORCE_RACE_OPTIONS,
    loadSpec: () => import('../air-force-size/airForceSizeRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-force-size', view: 'race',
      projection: mod.airForceSizeRaceProjection, declaration: mod.airForceSizeRaceDeclaration,
    })),
  },
  {
    kind: 'air-kills-losses',
    view: 'butterfly',
    label: 'Butterfly',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../air-kills-losses/airKillsLossesProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-kills-losses', view: 'butterfly',
      projection: mod.airKillsLossesProjection, declaration: mod.airKillsLossesDeclaration,
    })),
  },
  {
    kind: 'air-kills-losses',
    view: 'paired',
    label: 'Paired bars',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../air-kills-losses/airKillsLossesProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-kills-losses', view: 'paired',
      projection: mod.airKillsLossesPairedProjection, declaration: mod.airKillsLossesPairedDeclaration,
    })),
  },
  {
    kind: 'air-superiority-region',
    view: DEFAULT_VIEW,
    family: 'bars',
    loadSpec: () => import('../air-superiority-region/airSuperiorityProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-superiority-region', view: DEFAULT_VIEW,
      projection: mod.airSuperiorityRegionProjection, declaration: mod.airSuperiorityRegionDeclaration,
    })),
  },
  {
    kind: 'bombing-damage',
    view: DEFAULT_VIEW,
    family: 'bars',
    loadSpec: () => import('../bombing-damage/bombingDamageProjection.ts').then(mod => ({
      family: 'bars', kind: 'bombing-damage', view: DEFAULT_VIEW,
      projection: mod.bombingDamageProjection, declaration: mod.bombingDamageDeclaration,
    })),
  },
  {
    kind: 'casualties',
    view: 'butterfly',
    label: 'Butterfly',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../casualties/casualtiesProjection.ts').then(mod => ({
      family: 'bars', kind: 'casualties', view: 'butterfly',
      projection: mod.casualtiesButterflyProjection, declaration: mod.casualtiesButterflyDeclaration,
    })),
  },
  {
    kind: 'casualties',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../casualties/casualtiesProjection.ts').then(mod => ({
      family: 'bars', kind: 'casualties', view: 'race',
      projection: mod.casualtiesRaceProjection, declaration: mod.casualtiesRaceDeclaration,
    })),
  },
  {
    kind: 'casualties-between',
    view: 'opponents',
    label: 'By opponent',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../casualties-between/casualtiesBetweenProjection.ts').then(mod => ({
      family: 'bars', kind: 'casualties-between', view: 'opponents',
      projection: mod.casualtyOpponentsProjection, declaration: mod.casualtyOpponentsDeclaration,
    })),
  },
  {
    kind: 'factories',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../factories/factoriesProjection.ts').then(mod => ({
      family: 'bars', kind: 'factories', view: 'race',
      projection: mod.factoriesProjection, declaration: mod.factoriesDeclaration,
    })),
  },
  {
    kind: 'game-score',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: GAME_SCORE_RACE_OPTIONS,
    loadSpec: () => import('../game-score/gameScoreRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'game-score', view: 'race',
      projection: mod.gameScoreRaceProjection, declaration: mod.gameScoreRaceDeclaration,
    })),
  },
  {
    kind: 'lend-lease-balance',
    view: DEFAULT_VIEW,
    family: 'bars',
    loadSpec: () => import('../lend-lease-balance/lendLeaseBalanceProjection.ts').then(mod => ({
      family: 'bars', kind: 'lend-lease-balance', view: DEFAULT_VIEW,
      projection: mod.lendLeaseBalanceProjection, declaration: mod.lendLeaseBalanceDeclaration,
    })),
  },
  {
    kind: 'minefields',
    view: DEFAULT_VIEW,
    family: 'bars',
    loadSpec: () => import('../minefields/minefieldsProjection.ts').then(mod => ({
      family: 'bars', kind: 'minefields', view: DEFAULT_VIEW,
      projection: mod.minefieldsProjection, declaration: mod.minefieldsDeclaration,
    })),
  },
  {
    kind: 'naval-losses',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: NAVAL_LOSSES_RACE_OPTIONS,
    loadSpec: () => import('../naval-losses/navalLossesRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'naval-losses', view: 'race',
      projection: mod.navalLossesRaceProjection, declaration: mod.navalLossesRaceDeclaration,
    })),
  },
  {
    kind: 'naval-supremacy',
    view: 'shares',
    label: 'Shares',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../naval-supremacy/navalSupremacyProjection.ts').then(m => ({
      family: 'bars', kind: 'naval-supremacy', view: 'shares',
      projection: m.supremacySharesProjection, declaration: m.supremacySharesDeclaration,
    })),
  },
  {
    kind: 'navy-size',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: NAVY_SIZE_RACE_OPTIONS,
    loadSpec: () => import('../navy-size/navySizeRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'navy-size', view: 'race',
      projection: mod.navySizeRaceProjection, declaration: mod.navySizeRaceDeclaration,
    })),
  },
  {
    kind: 'nuclear-arsenal',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: NUCLEAR_ARSENAL_RACE_OPTIONS,
    loadSpec: () => import('../nuclear-arsenal/nuclearArsenalRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'nuclear-arsenal', view: 'race',
      projection: mod.nuclearArsenalRaceProjection, declaration: mod.nuclearArsenalRaceDeclaration,
    })),
  },
  {
    kind: 'occupation-trend',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: OCCUPATION_TREND_RACE_OPTIONS,
    loadSpec: () => import('../occupation-trend/occupationTrendRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'occupation-trend', view: 'race',
      projection: mod.occupationTrendRaceProjection, declaration: mod.occupationTrendRaceDeclaration,
    })),
  },
  {
    kind: 'who-spies-on-whom',
    view: 'pairs',
    label: 'Top pairs',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../who-spies-on-whom/whoSpiesProjection.ts').then(mod => ({
      family: 'bars', kind: 'who-spies-on-whom', view: 'pairs',
      projection: mod.operativePairsProjection, declaration: mod.operativePairsDeclaration,
    })),
  },
  {
    kind: 'world-tension',
    view: 'race',
    label: 'Contributions race',
    family: 'bars',
    viewOption: 'view',
    options: WORLD_TENSION_RACE_OPTIONS,
    loadSpec: () => import('../world-tension/worldTensionRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'world-tension', view: 'race',
      projection: mod.worldTensionRaceProjection, declaration: mod.worldTensionRaceDeclaration,
    })),
  },
];

export const barsFamilyManifest: FamilyManifestModule = {
  family: 'bars',
  laneView: familyBarsLaneView,
  views: BARS_VIEWS,
  hostAdapter: barsHostAdapter,
};
