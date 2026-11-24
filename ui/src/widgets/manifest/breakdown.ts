import { breakdownDeclaration, type BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import { breakdownHostAdapter } from '../../families/breakdown/hostAdapter.ts';
import { familyBreakdownLaneView } from '../../families/breakdown/laneView.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';

function breakdownView(kind: string, load: () => Promise<BreakdownWidgetSpec<any>>): FamilyViewEntry {
  return {
    kind,
    view: DEFAULT_VIEW,
    family: 'breakdown',
    loadSpec: () => load().then(spec => ({
      family: 'breakdown',
      kind,
      view: DEFAULT_VIEW,
      projection: spec.projection,
      declaration: breakdownDeclaration(spec),
    })),
  };
}

export const BREAKDOWN_FAMILY_VIEWS: readonly FamilyViewEntry[] = [
  breakdownView(
    'political-power-spending-breakdown',
    () => import('../political-power-spending-breakdown/ppSpendingProjection.ts').then(mod => mod.ppSpendingBreakdown),
  ),
  breakdownView('construction-spending', () => import('../construction-spending/constructionProjection.ts').then(mod => mod.constructionBreakdown)),
];

export const breakdownFamilyManifest: FamilyManifestModule = {
  family: 'breakdown',
  laneView: familyBreakdownLaneView,
  views: BREAKDOWN_FAMILY_VIEWS,
  hostAdapter: breakdownHostAdapter,
};
