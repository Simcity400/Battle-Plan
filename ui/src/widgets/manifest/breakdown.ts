import type { OptionField } from '../../dashboard/options.ts';
import { breakdownDeclaration, type BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import { breakdownHostAdapter } from '../../families/breakdown/hostAdapter.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';
import { OPTIONS as TERRITORY_OPTIONS } from '../territory/definition.ts';

const TERRITORY_SHARE_OPTIONS: readonly OptionField[] = [
  ...TERRITORY_OPTIONS.filter(field => field.key === 'view' || field.key === 'measure'),
  { key: 'rule', section: 'options', label: 'Territory rule', kind: 'choice', default: 'controlled',
    choices: [{ value: 'controlled', label: 'Controlled' }, { value: 'owned', label: 'Owned' }] },
];

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
  breakdownView('casualties-breakdown', () => import('../casualties-breakdown/casualtiesBreakdownProjection.ts').then(mod => mod.casualtiesBreakdown)),
  {
    kind: 'territory',
    view: 'share',
    label: 'Share',
    family: 'breakdown',
    viewOption: 'view',
    options: TERRITORY_SHARE_OPTIONS,
    loadSpec: () => import('../territory/territoryShareProjection.ts').then(mod => {
      const declaration = breakdownDeclaration(mod.territoryShare);
      return { family: 'breakdown', kind: 'territory', view: 'share', projection: declaration.projection, declaration };
    }),
  },
];

export const breakdownFamilyManifest: FamilyManifestModule = {
  family: 'breakdown',
  lane: () => import('../../families/breakdown/laneView.ts').then(m => m.breakdownLane),
  views: BREAKDOWN_FAMILY_VIEWS,
  hostAdapter: breakdownHostAdapter,
};
