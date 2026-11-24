import type { WidgetRenderer } from '../../dashboard/definitions';
import { useCategoryChoices } from './categoryChoices';
import { useIdeaTimelinePresets } from './ideaTimelinePresets';
import { ideaTimelineDeclaration } from './ideaTimelineProjection';

export default {
  declaration: ideaTimelineDeclaration,
  useOptionChoices: useCategoryChoices,
  usePresets: useIdeaTimelinePresets,
} satisfies WidgetRenderer;
