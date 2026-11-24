import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { familyBarsLaneView } from '../../families/bars/laneView.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';

export const BARS_VIEWS: readonly FamilyViewEntry[] = [
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
];

export const barsFamilyManifest: FamilyManifestModule = {
  family: 'bars',
  laneView: familyBarsLaneView,
  views: BARS_VIEWS,
  hostAdapter: barsHostAdapter,
};
