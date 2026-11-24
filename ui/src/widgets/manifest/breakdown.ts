import { breakdownDeclaration, type BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import { breakdownHostAdapter } from '../../families/breakdown/hostAdapter.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';

function breakdownView(kind: string, load: () => Promise<BreakdownWidgetSpec<any>>): FamilyViewEntry {
  return {
    kind,
    view: DEFAULT_VIEW,
    family: 'breakdown',
    loadSpec: () => load().then(spec => {
      const declaration = breakdownDeclaration(spec);
      return { family: 'breakdown', kind, view: DEFAULT_VIEW, projection: declaration.projection, declaration };
    }),
  };
}

export const BREAKDOWN_FAMILY_VIEWS: readonly FamilyViewEntry[] = [
  breakdownView(
    'political-power-spending-breakdown',
    () => import('../political-power-spending-breakdown/ppSpendingProjection.ts').then(mod => mod.ppSpendingBreakdown),
  ),
  breakdownView('construction-spending', () => import('../construction-spending/constructionProjection.ts').then(mod => mod.constructionBreakdown)),
  breakdownView('territory-breakdown', () => import('../territory-breakdown/territoryBreakdownProjection.ts').then(mod => mod.territoryBreakdown)),
];

export const breakdownFamilyManifest: FamilyManifestModule = {
  family: 'breakdown',
  lane: () => import('../../families/breakdown/laneView.ts').then(m => m.breakdownLane),
  views: BREAKDOWN_FAMILY_VIEWS,
  hostAdapter: breakdownHostAdapter,
};
