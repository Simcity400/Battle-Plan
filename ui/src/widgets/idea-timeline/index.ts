import type { WidgetRenderer } from '../../dashboard/definitions';
import IdeaTimelineWidget from './IdeaTimelineWidget';
import IdeaTimelinePickerItems from './IdeaTimelinePickerItems';
import { useCategoryChoices } from './categoryChoices';

export default { render: IdeaTimelineWidget, pickerItems: IdeaTimelinePickerItems, useOptionChoices: useCategoryChoices } satisfies WidgetRenderer;
