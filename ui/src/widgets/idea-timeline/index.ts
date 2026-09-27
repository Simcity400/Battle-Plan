import type { WidgetRenderer } from '../../dashboard/definitions';
import IdeaTimelineWidget from './IdeaTimelineWidget';
import { useCategoryChoices } from './categoryChoices';

export default { render: IdeaTimelineWidget, useOptionChoices: useCategoryChoices } satisfies WidgetRenderer;
