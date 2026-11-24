import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry, type FamilyViewModule } from '../../families/manifest.ts';
import type { AnyTimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { timelineHostAdapter } from '../../families/timeline/hostAdapter.ts';
import { familyTimelineLaneView } from '../../families/timeline/laneView.ts';
import type { TimelineProjection } from '../../families/timeline/model.ts';

function timelineView<TModule>(
  kind: string,
  load: () => Promise<TModule>,
  pick: (mod: TModule) => { readonly projection: TimelineProjection<any>; readonly declaration: AnyTimelineWidgetDeclaration },
): FamilyViewEntry {
  return {
    kind,
    view: DEFAULT_VIEW,
    family: 'timeline',
    loadSpec: (): Promise<FamilyViewModule> => load().then(mod => {
      const { projection, declaration } = pick(mod);
      return { family: 'timeline', kind, view: DEFAULT_VIEW, projection, declaration };
    }),
  };
}

export const TIMELINE_VIEWS: readonly FamilyViewEntry[] = [
  timelineView('diplomatic-pacts', () => import('../diplomatic-pacts/diplomaticPactsProjection.ts'),
    mod => ({ projection: mod.diplomaticPactsProjection, declaration: mod.diplomaticPactsDeclaration })),
  timelineView('factions', () => import('../factions/factionsProjection.ts'),
    mod => ({ projection: mod.factionsProjection, declaration: mod.factionsDeclaration })),
  timelineView('idea-timeline', () => import('../idea-timeline/ideaTimelineProjection.ts'),
    mod => ({ projection: mod.ideaTimelineProjection, declaration: mod.ideaTimelineDeclaration })),
  timelineView('spirit-history', () => import('../spirit-history/spiritHistoryProjection.ts'),
    mod => ({ projection: mod.spiritHistoryProjection, declaration: mod.spiritHistoryDeclaration })),
  timelineView('subjects', () => import('../subjects/subjectsProjection.ts'),
    mod => ({ projection: mod.subjectsProjection, declaration: mod.subjectsDeclaration })),
];

export const timelineFamilyManifest: FamilyManifestModule = {
  family: 'timeline',
  laneView: familyTimelineLaneView,
  views: TIMELINE_VIEWS,
  hostAdapter: timelineHostAdapter,
};
