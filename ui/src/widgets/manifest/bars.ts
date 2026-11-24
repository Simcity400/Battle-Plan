import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { familyBarsLaneView } from '../../families/bars/laneView.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';
import { OPTIONS as GAME_SCORE_OPTIONS } from '../game-score/definition.ts';
import { OPTIONS as OCCUPATION_TREND_OPTIONS } from '../occupation-trend/definition.ts';
import { OPTIONS as WORLD_TENSION_OPTIONS } from '../world-tension/definition.ts';

const GAME_SCORE_RACE_OPTION_KEYS = new Set(['view', 'topMeasure', 'split']);
const GAME_SCORE_RACE_OPTIONS = GAME_SCORE_OPTIONS.filter(field => GAME_SCORE_RACE_OPTION_KEYS.has(field.key));
const OCCUPATION_TREND_RACE_OPTION_KEYS = new Set(['view', 'measure', 'rankingMeasure', 'digits']);
const OCCUPATION_TREND_RACE_OPTIONS = OCCUPATION_TREND_OPTIONS.filter(field => OCCUPATION_TREND_RACE_OPTION_KEYS.has(field.key));
const WORLD_TENSION_RACE_OPTIONS = WORLD_TENSION_OPTIONS.filter(field => field.key === 'view');

export const BARS_VIEWS: readonly FamilyViewEntry[] = [
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
