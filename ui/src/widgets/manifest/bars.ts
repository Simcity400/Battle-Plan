import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { familyBarsLaneView } from '../../families/bars/laneView.ts';
import type { FamilyManifestModule, FamilyViewEntry } from '../../families/manifest.ts';

export const BARS_VIEWS: readonly FamilyViewEntry[] = [
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
];

export const barsFamilyManifest: FamilyManifestModule = {
  family: 'bars',
  laneView: familyBarsLaneView,
  views: BARS_VIEWS,
  hostAdapter: barsHostAdapter,
};
