import type { OptionField } from '../../dashboard/options.ts';
import { breakdownDeclaration, type BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';
import { OPTIONS as DOCKYARD_OPTIONS } from '../dockyard-spending/definition.ts';
import { OPTIONS as FACTORY_OPTIONS } from '../military-factory-spending/definition.ts';
import { OPTIONS as TERRITORY_OPTIONS } from '../territory/definition.ts';

const TERRITORY_SHARE_OPTIONS: readonly OptionField[] = TERRITORY_OPTIONS.filter(field => field.key === 'view' || field.key === 'measure' || field.key === 'rule');

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

function breakdownNamedView(kind: string, view: string, spec: BreakdownWidgetSpec<any>) {
  const declaration = breakdownDeclaration(spec);
  return { family: 'breakdown' as const, kind, view, projection: declaration.projection, declaration };
}

export const BREAKDOWN_FAMILY_VIEWS: readonly FamilyViewEntry[] = [
  breakdownView(
    'political-power-spending-breakdown',
    () => import('../political-power-spending-breakdown/ppSpendingProjection.ts').then(mod => mod.ppSpendingBreakdown),
  ),
  breakdownView('construction-spending', () => import('../construction-spending/constructionProjection.ts').then(mod => mod.constructionBreakdown)),
  breakdownView('territory-breakdown', () => import('../territory-breakdown/territoryBreakdownProjection.ts').then(mod => mod.territoryBreakdown)),
  breakdownView('casualties-breakdown', () => import('../casualties-breakdown/casualtiesBreakdownProjection.ts').then(mod => mod.casualtiesBreakdown)),
  breakdownView('production-spending', () => import('../production-spending/productionSpendingProjection.ts').then(mod => mod.productionSpendingBreakdown)),
  breakdownView('equipment-losses', () => import('../equipment-losses/equipmentLossesProjection.ts').then(mod => mod.equipmentLossesBreakdown)),
  breakdownView('bombing-damage-breakdown', () => import('../bombing-damage-breakdown/bombingBreakdownProjection.ts').then(mod => mod.bombingDamageBreakdown)),
  {
    kind: 'dockyard-spending',
    view: 'parts',
    label: 'Parts',
    family: 'breakdown',
    viewOption: 'view',
    options: DOCKYARD_OPTIONS,
    loadSpec: () => import('../production-spending/productionSpendingProjection.ts').then(mod => breakdownNamedView('dockyard-spending', 'parts', mod.dockyardSpendingBreakdown)),
  },
  {
    kind: 'dockyard-spending',
    view: 'ring',
    label: 'Ring',
    family: 'breakdown',
    viewOption: 'view',
    options: DOCKYARD_OPTIONS,
    loadSpec: () => import('../production-spending/productionSpendingProjection.ts').then(mod => breakdownNamedView('dockyard-spending', 'ring', mod.dockyardSpendingBreakdown)),
  },
  {
    kind: 'dockyard-spending',
    view: 'mosaic',
    label: 'Mosaic',
    family: 'breakdown',
    viewOption: 'view',
    options: DOCKYARD_OPTIONS,
    loadSpec: () => import('../production-spending/productionSpendingProjection.ts').then(mod => breakdownNamedView('dockyard-spending', 'mosaic', mod.dockyardSpendingBreakdown)),
  },
  {
    kind: 'military-factory-spending',
    view: 'parts',
    label: 'Parts',
    family: 'breakdown',
    viewOption: 'view',
    options: FACTORY_OPTIONS,
    loadSpec: () => import('../production-spending/productionSpendingProjection.ts').then(mod => breakdownNamedView('military-factory-spending', 'parts', mod.militaryFactorySpendingBreakdown)),
  },
  {
    kind: 'military-factory-spending',
    view: 'ring',
    label: 'Ring',
    family: 'breakdown',
    viewOption: 'view',
    options: FACTORY_OPTIONS,
    loadSpec: () => import('../production-spending/productionSpendingProjection.ts').then(mod => breakdownNamedView('military-factory-spending', 'ring', mod.militaryFactorySpendingBreakdown)),
  },
  {
    kind: 'military-factory-spending',
    view: 'mosaic',
    label: 'Mosaic',
    family: 'breakdown',
    viewOption: 'view',
    options: FACTORY_OPTIONS,
    loadSpec: () => import('../production-spending/productionSpendingProjection.ts').then(mod => breakdownNamedView('military-factory-spending', 'mosaic', mod.militaryFactorySpendingBreakdown)),
  },
  {
    kind: 'territory',
    view: 'share',
    label: 'Share',
    family: 'breakdown',
    viewOption: 'view',
    options: TERRITORY_SHARE_OPTIONS,
    loadSpec: () => import('../territory/territoryShareProjection.ts').then(mod => breakdownNamedView('territory', 'share', mod.territoryShare)),
  },
];

export const breakdownFamilyManifest: FamilyManifestModule = {
  family: 'breakdown',
  lane: () => import('../../families/breakdown/laneView.ts').then(m => m.breakdownLane),
  views: BREAKDOWN_FAMILY_VIEWS,
};
