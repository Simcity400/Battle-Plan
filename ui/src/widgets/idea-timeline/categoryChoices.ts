import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import { useIdeaHistory } from '../../dashboard/ideaHistory';
import type { WidgetSpec } from '../../dashboard/model';
import { timelineCategories } from './ideaTimelineData';

export function useCategoryChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useIdeaHistory(widget, runtime, true);
  return { category: timelineCategories(data?.categories ?? []).map(category => ({ value: category.token, label: contentTitle(category.name, category.token, 'Category name unavailable') })) };
}
