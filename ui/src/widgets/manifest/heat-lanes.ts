import { heatLanesHostAdapter } from '../../families/heat/hostAdapter.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';

export const HEAT_LANES_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'naval-supremacy',
    view: 'timeline',
    label: 'Timeline',
    family: 'heat-lanes',
    viewOption: 'view',
    loadSpec: () => import('../naval-supremacy/navalSupremacyProjection.ts').then(m => ({
      family: 'heat-lanes', kind: 'naval-supremacy', view: 'timeline',
      projection: m.supremacyTimelineProjection, declaration: m.supremacyTimelineDeclaration,
    })),
  },
  {
    kind: 'weather-by-region',
    view: DEFAULT_VIEW,
    family: 'heat-lanes',
    loadSpec: () => import('../weather-by-region/weatherProjection.ts').then(m => ({
      family: 'heat-lanes', kind: 'weather-by-region', view: DEFAULT_VIEW,
      projection: m.weatherProjection, declaration: m.weatherDeclaration,
    })),
  },
  {
    kind: 'air-superiority-timeline',
    view: DEFAULT_VIEW,
    family: 'heat-lanes',
    loadSpec: () => import('../air-superiority-timeline/airTimelineProjection.ts').then(m => ({
      family: 'heat-lanes', kind: 'air-superiority-timeline', view: DEFAULT_VIEW,
      projection: m.airTimelineProjection, declaration: m.airTimelineDeclaration,
    })),
  },
  {
    kind: 'convoy-health',
    view: DEFAULT_VIEW,
    family: 'heat-lanes',
    loadSpec: () => import('../convoy-health/convoyHealthProjection.ts').then(m => ({
      family: 'heat-lanes', kind: 'convoy-health', view: DEFAULT_VIEW,
      projection: m.convoyHealthProjection, declaration: m.convoyHealthDeclaration,
    })),
  },
];

export const heatLanesFamilyManifest: FamilyManifestModule = {
  family: 'heat-lanes',
  lane: () => import('../../families/heat/lanesLaneView.ts').then(m => m.heatLanesLane),
  views: HEAT_LANES_VIEWS,
  hostAdapter: heatLanesHostAdapter,
};
