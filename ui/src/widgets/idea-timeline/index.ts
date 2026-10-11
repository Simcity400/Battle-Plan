import type { WidgetRenderer } from '../../dashboard/definitions';
import IdeaTimelineWidget from './IdeaTimelineWidget';
import { useIdeaTimelinePresets } from './ideaTimelinePresets';
import { useCategoryChoices } from './categoryChoices';

export default { render: IdeaTimelineWidget, usePresets: useIdeaTimelinePresets, useOptionChoices: useCategoryChoices } satisfies WidgetRenderer;
