import type { FamilyManifestModule, FamilyViewEntry } from '../families/manifest.ts';
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

bindFamilyManifest(FAMILY_VIEWS);
