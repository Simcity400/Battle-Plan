import { familyHeatMatrixLaneView } from '../../families/heat/matrixLaneView.ts';
import type { FamilyManifestModule, FamilyViewEntry } from '../../families/manifest.ts';

export const HEAT_MATRIX_VIEWS: readonly FamilyViewEntry[] = [];

export const heatMatrixFamilyManifest: FamilyManifestModule = {
  family: 'heat-matrix',
  laneView: familyHeatMatrixLaneView,
  views: HEAT_MATRIX_VIEWS,
};
