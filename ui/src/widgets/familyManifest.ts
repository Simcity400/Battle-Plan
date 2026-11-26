import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../families/manifest.ts';
import { bindFamilyManifest } from '../families/registry.ts';
import { barsFamilyManifest } from './manifest/bars.ts';
import { breakdownFamilyManifest } from './manifest/breakdown.ts';
import { cardFamilyManifest } from './manifest/card.ts';
import { flowFamilyManifest } from './manifest/flow.ts';
import { heatLanesFamilyManifest } from './manifest/heat-lanes.ts';
import { heatMatrixFamilyManifest } from './manifest/heat-matrix.ts';
import { metersFamilyManifest } from './manifest/meters.ts';
import { tableFamilyManifest } from './manifest/table.ts';
import { timeChartFamilyManifest } from './manifest/time-chart.ts';
import { timelineFamilyManifest } from './manifest/timeline.ts';

export const FAMILY_MANIFESTS: readonly FamilyManifestModule[] = [
  barsFamilyManifest,
  breakdownFamilyManifest,
  cardFamilyManifest,
  flowFamilyManifest,
  heatLanesFamilyManifest,
  heatMatrixFamilyManifest,
  metersFamilyManifest,
  tableFamilyManifest,
  timeChartFamilyManifest,
  timelineFamilyManifest,
];

export const FAMILY_VIEWS: readonly FamilyViewEntry[] = FAMILY_MANIFESTS.flatMap(entry => entry.views);

const DETAIL_FAMILY_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'fuel-accounting-table',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: () => import('./fuel/fuelDetailProjection.ts').then(mod => ({
      family: 'table',
      kind: 'fuel-accounting-table',
      view: DEFAULT_VIEW,
      projection: mod.fuelAccountingTableProjection,
      declaration: mod.fuelAccountingTableDeclaration,
    })),
  },
  {
    kind: 'fuel-detail-table',
    view: DEFAULT_VIEW,
    family: 'table',
    loadSpec: () => import('./fuel/fuelDetailProjection.ts').then(mod => ({
      family: 'table',
      kind: 'fuel-detail-table',
      view: DEFAULT_VIEW,
      projection: mod.fuelDetailTableProjection,
      declaration: mod.fuelDetailTableDeclaration,
    })),
  },
  {
    kind: 'fuel-series-chart',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: () => import('./fuel/fuelDetailProjection.ts').then(mod => ({
      family: 'time-chart',
      kind: 'fuel-series-chart',
      view: DEFAULT_VIEW,
      projection: mod.fuelSeriesChartProjection,
      declaration: mod.fuelSeriesChartDeclaration,
    })),
  },
  {
    kind: 'fuel-history-chart',
    view: DEFAULT_VIEW,
    family: 'time-chart',
    loadSpec: () => import('./fuel/fuelDetailProjection.ts').then(mod => ({
      family: 'time-chart',
      kind: 'fuel-history-chart',
      view: DEFAULT_VIEW,
      projection: mod.fuelHistoryChartProjection,
      declaration: mod.fuelHistoryChartDeclaration,
    })),
  },
];

bindFamilyManifest(FAMILY_VIEWS);
bindFamilyManifest(DETAIL_FAMILY_VIEWS);
