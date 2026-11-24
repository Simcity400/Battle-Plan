import { breakdownDeclaration, type BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import { breakdownHostAdapter } from '../../families/breakdown/hostAdapter.ts';
import { familyBreakdownLaneView } from '../../families/breakdown/laneView.ts';
import { BREAKDOWN_VIEWS } from '../../families/breakdown/options.ts';
import type { FamilyManifestModule, FamilyViewEntry } from '../../families/manifest.ts';

function breakdownViews(kind: string, load: () => Promise<BreakdownWidgetSpec<any>>): readonly FamilyViewEntry[] {
  return BREAKDOWN_VIEWS.map(({ value: view, label }) => ({
    kind,
    view,
    label,
    family: 'breakdown',
    viewOption: 'view',
    loadSpec: () => load().then(spec => ({
      family: 'breakdown',
      kind,
      view,
      projection: spec.projection,
      declaration: breakdownDeclaration(spec, view),
    })),
  }));
}

export const BREAKDOWN_FAMILY_VIEWS: readonly FamilyViewEntry[] = [
  ...breakdownViews(
    'political-power-spending-breakdown',
    () => import('../political-power-spending-breakdown/ppSpendingProjection.ts').then(mod => mod.ppSpendingBreakdown),
  ),
];

export const breakdownFamilyManifest: FamilyManifestModule = {
  family: 'breakdown',
  laneView: familyBreakdownLaneView,
  views: BREAKDOWN_FAMILY_VIEWS,
  hostAdapter: breakdownHostAdapter,
};
