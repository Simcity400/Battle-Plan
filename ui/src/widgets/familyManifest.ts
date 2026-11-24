import type { LaneView } from '../draw/lane.ts';
import type { FamilyManifestModule, FamilyViewEntry } from '../families/manifest.ts';
import { bindFamilyManifest } from '../families/registry.ts';
import { barsFamilyManifest } from './manifest/bars.ts';
import { cardFamilyManifest } from './manifest/card.ts';
import { heatLanesFamilyManifest } from './manifest/heat-lanes.ts';
import { heatMatrixFamilyManifest } from './manifest/heat-matrix.ts';
import { tableFamilyManifest } from './manifest/table.ts';
import { timeChartFamilyManifest } from './manifest/time-chart.ts';

export const FAMILY_MANIFESTS: readonly FamilyManifestModule[] = [
  barsFamilyManifest,
  cardFamilyManifest,
  heatLanesFamilyManifest,
  heatMatrixFamilyManifest,
  tableFamilyManifest,
  timeChartFamilyManifest,
];

export const FAMILY_VIEWS: readonly FamilyViewEntry[] = FAMILY_MANIFESTS.flatMap(entry => entry.views);

const laneViewFactories = new Map<string, (wake: (widget: number) => void) => LaneView>(
  FAMILY_MANIFESTS.map(entry => [entry.family, entry.laneView]),
);

export function familyLaneView(family: string, wake: (widget: number) => void): LaneView | undefined {
  return laneViewFactories.get(family)?.(wake);
}

bindFamilyManifest(FAMILY_VIEWS);
