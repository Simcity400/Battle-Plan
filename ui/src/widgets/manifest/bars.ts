import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { familyBarsLaneView } from '../../families/bars/laneView.ts';
import type { FamilyManifestModule, FamilyViewEntry } from '../../families/manifest.ts';

export const BARS_VIEWS: readonly FamilyViewEntry[] = [];

export const barsFamilyManifest: FamilyManifestModule = {
  family: 'bars',
  laneView: familyBarsLaneView,
  views: BARS_VIEWS,
  hostAdapter: barsHostAdapter,
};
