import { heatMatrixHostAdapter } from '../../families/heat/hostAdapter.ts';
import { familyHeatMatrixLaneView } from '../../families/heat/matrixLaneView.ts';
import type { FamilyManifestModule, FamilyViewEntry } from '../../families/manifest.ts';
import { OPTIONS as CASUALTIES_BETWEEN_OPTIONS } from '../casualties-between/definition.ts';

export const HEAT_MATRIX_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'who-spies-on-whom',
    view: 'matrix',
    label: 'Matrix',
    family: 'heat-matrix',
    viewOption: 'view',
    loadSpec: () => import('../who-spies-on-whom/whoSpiesProjection.ts').then(mod => ({
      family: 'heat-matrix', kind: 'who-spies-on-whom', view: 'matrix',
      projection: mod.operativeMatrixProjection, declaration: mod.operativeMatrixDeclaration,
    })),
  },
  {
    kind: 'casualties-between',
    view: 'matrix',
    label: 'Matrix',
    family: 'heat-matrix',
    viewOption: 'view',
    options: CASUALTIES_BETWEEN_OPTIONS.filter(field => field.key !== 'view'),
    loadSpec: () => import('../casualties-between/casualtiesBetweenProjection.ts').then(mod => ({
      family: 'heat-matrix', kind: 'casualties-between', view: 'matrix',
      projection: mod.casualtyMatrixProjection, declaration: mod.casualtyMatrixDeclaration,
    })),
  },
];

export const heatMatrixFamilyManifest: FamilyManifestModule = {
  family: 'heat-matrix',
  laneView: familyHeatMatrixLaneView,
  views: HEAT_MATRIX_VIEWS,
  hostAdapter: heatMatrixHostAdapter,
};
