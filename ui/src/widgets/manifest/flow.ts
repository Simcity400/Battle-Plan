import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';

export const FLOW_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'trade-flows',
    view: 'flow',
    label: 'Flow',
    family: 'flow',
    viewOption: 'view',
    loadSpec: () => import('../trade-flows/tradeFlowsProjection.ts').then(mod => ({
      family: 'flow', kind: 'trade-flows', view: 'flow',
      projection: mod.tradeFlowProjection, declaration: mod.tradeFlowDeclaration,
    })),
  },
  {
    kind: 'production-licences',
    view: DEFAULT_VIEW,
    family: 'flow',
    loadSpec: () => import('../production-licences/licencesProjection.ts').then(mod => ({
      family: 'flow', kind: 'production-licences', view: DEFAULT_VIEW,
      projection: mod.licencesProjection, declaration: mod.licencesDeclaration,
    })),
  },
  {
    kind: 'volunteers',
    view: DEFAULT_VIEW,
    family: 'flow',
    loadSpec: () => import('../volunteers/volunteersProjection.ts').then(mod => ({
      family: 'flow', kind: 'volunteers', view: DEFAULT_VIEW,
      projection: mod.volunteersProjection, declaration: mod.volunteersDeclaration,
    })),
  },
];

export const flowFamilyManifest: FamilyManifestModule = {
  family: 'flow',
  lane: () => import('../../families/flow/laneView.ts').then(m => m.flowLane),
  views: FLOW_VIEWS,
};
