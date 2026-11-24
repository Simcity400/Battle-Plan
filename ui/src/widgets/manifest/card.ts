import { cardHostAdapter } from '../../families/card/hostAdapter.ts';
import { familyCardLaneView } from '../../families/card/laneView.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';

export const CARD_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'national-focus',
    view: DEFAULT_VIEW,
    family: 'card',
    loadSpec: () => import('../national-focus/nationalFocusProjection.ts').then(m => ({
      family: 'card', kind: 'national-focus', view: DEFAULT_VIEW,
      projection: m.nationalFocusProjection, declaration: m.nationalFocusDeclaration,
    })),
  },
  {
    kind: 'national-spirits',
    view: DEFAULT_VIEW,
    family: 'card',
    loadSpec: () => import('../national-spirits/nationalSpiritsProjection.ts').then(m => ({
      family: 'card', kind: 'national-spirits', view: DEFAULT_VIEW,
      projection: m.nationalSpiritsProjection, declaration: m.nationalSpiritsDeclaration,
    })),
  },
  {
    kind: 'special-projects',
    view: 'live',
    label: 'Current',
    family: 'card',
    viewOption: 'view',
    loadSpec: () => import('../special-projects/specialProjectsProjection.ts').then(m => ({
      family: 'card', kind: 'special-projects', view: 'live',
      projection: m.specialProjectsProjection, declaration: m.specialProjectsDeclaration,
    })),
  },
  {
    kind: 'war-goals',
    view: DEFAULT_VIEW,
    family: 'card',
    loadSpec: () => import('../war-goals/warGoalsProjection.ts').then(m => ({
      family: 'card', kind: 'war-goals', view: DEFAULT_VIEW,
      projection: m.warGoalsProjection, declaration: m.warGoalsDeclaration,
    })),
  },
];

export const cardFamilyManifest: FamilyManifestModule = {
  family: 'card',
  laneView: familyCardLaneView,
  views: CARD_VIEWS,
  hostAdapter: cardHostAdapter,
};
