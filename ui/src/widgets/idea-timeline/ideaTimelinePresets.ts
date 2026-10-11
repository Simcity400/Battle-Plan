import { useMemo } from 'react';
import type { WidgetMeta, WidgetPresets, WidgetRuntime } from '../../dashboard/definitions';
import { useIdeaHistory } from '../../dashboard/ideaHistory';
import { createWidget } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import { timelineCategories } from './ideaTimelineData';
import { contentTitle } from '../../dashboard/contentTitles.ts';

const NONE: WidgetPresets = { presets: [], reason: null };

/** One catalogue entry per idea category the campaign's own content shows on the political screen. */
export function useIdeaTimelinePresets(definition: WidgetMeta, runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean): WidgetPresets {
  const widget = useMemo(() => createWidget(definition.kind, track?.id ?? '', 'idea-timeline-presets'), [definition.kind, track?.id]);
  const { tag, data, error } = useIdeaHistory(widget, runtime, active);
  if (!active) return NONE;
  const categories = data ? timelineCategories(data.categories) : [];
  const reason = !track ? 'Load a campaign to list its idea categories.'
    : !tag ? 'Choose a country to list this campaign’s idea categories.'
    : error ?? (!data ? 'Loading this campaign’s idea categories…'
      : !categories.length ? 'This campaign’s content has no political-screen idea categories with slots.' : null);
  if (reason !== null) return { presets: [], reason };
  return { reason: null, presets: categories.map(category => {
    // A known empty name keeps the widget's own editable title rather than an invented one.
    const name = contentTitle(category.name, category.token, 'Category name unavailable');
    return { key: category.token, initial: { title: name || definition.title, options: { category: category.token } } };
  }) };
}
