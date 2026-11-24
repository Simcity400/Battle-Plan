import { heatMatrixHostAdapter } from '../../families/heat/hostAdapter.ts';
import { familyHeatMatrixLaneView } from '../../families/heat/matrixLaneView.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';

export const HEAT_MATRIX_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'diplomatic-relations',
    view: DEFAULT_VIEW,
    family: 'heat-matrix',
    loadSpec: () => import('../diplomatic-relations/diplomaticRelationsProjection.ts').then(m => ({
      family: 'heat-matrix', kind: 'diplomatic-relations', view: DEFAULT_VIEW,
      projection: m.diplomaticRelationsProjection, declaration: m.diplomaticRelationsDeclaration,
    })),
  },
  {
    kind: 'intel-network-coverage',
    view: DEFAULT_VIEW,
    family: 'heat-matrix',
    loadSpec: () => import('../intel-network-coverage/intelCoverageProjection.ts').then(m => ({
      family: 'heat-matrix', kind: 'intel-network-coverage', view: DEFAULT_VIEW,
      projection: m.intelCoverageProjection, declaration: m.intelCoverageDeclaration,
    })),
  },
];

export const heatMatrixFamilyManifest: FamilyManifestModule = {
  family: 'heat-matrix',
  laneView: familyHeatMatrixLaneView,
  views: HEAT_MATRIX_VIEWS,
  hostAdapter: heatMatrixHostAdapter,
};
