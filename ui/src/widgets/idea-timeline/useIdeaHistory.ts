import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { IdeaHistoryReply } from '../../generated/dashboard';
import { timelineCategories, validateIdeaHistoryReply } from './ideaTimelineData';

/** The widget's country's idea history, shared by the widget and its settings. */
export function useIdeaHistory(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'idea-history',
    params: { branchId: track.branch.id, tag } } : null;
  return { track, tag, ...useProduct<IdeaHistoryReply>(runtime.client, request, active, validateIdeaHistoryReply) };
}

/** The Category setting's choices: the campaign's own categories, named as the game names them. */
export function useCategoryChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useIdeaHistory(widget, runtime, true);
  return { category: timelineCategories(data?.categories ?? []).map(category => ({ value: category.token, label: category.name ?? category.token })) };
}
