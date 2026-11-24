import { heatMatrixHostAdapter } from '../../families/heat/hostAdapter.ts';
import { familyHeatMatrixLaneView } from '../../families/heat/matrixLaneView.ts';
import type { FamilyManifestModule, FamilyViewEntry } from '../../families/manifest.ts';

export const HEAT_MATRIX_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'decryption',
    view: 'matrix',
    label: 'Who reads whom',
    family: 'heat-matrix',
    viewOption: 'view',
    loadSpec: () => import('../decryption/whoReadsWhomProjection.ts').then(mod => ({
      family: 'heat-matrix',
      kind: 'decryption',
      view: 'matrix',
      projection: mod.whoReadsWhomProjection,
      declaration: mod.whoReadsWhomDeclaration,
    })),
  },
];

export const heatMatrixFamilyManifest: FamilyManifestModule = {
  family: 'heat-matrix',
  laneView: familyHeatMatrixLaneView,
  views: HEAT_MATRIX_VIEWS,
  hostAdapter: heatMatrixHostAdapter,
};
