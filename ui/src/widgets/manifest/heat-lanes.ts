import { familyHeatLanesLaneView } from '../../families/heat/lanesLaneView.ts';
import type { FamilyManifestModule, FamilyViewEntry } from '../../families/manifest.ts';

export const HEAT_LANES_VIEWS: readonly FamilyViewEntry[] = [];

export const heatLanesFamilyManifest: FamilyManifestModule = {
  family: 'heat-lanes',
  laneView: familyHeatLanesLaneView,
  views: HEAT_LANES_VIEWS,
};
