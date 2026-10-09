import { useEffect, useMemo, useRef } from 'react';
import type { WidgetPickerItemsProps } from '../../dashboard/definitions';
import { useIdeaHistory } from '../../dashboard/ideaHistory';
import { createWidget } from '../../dashboard/model';
import WidgetPickerItem from '../../dashboard/WidgetPickerItem';
import { timelineCategories } from './ideaTimelineData';

/** Each eligible content category starts the same timeline with its own choice. */
export default function IdeaTimelinePickerItems({ definition, runtime, track, active, disabled, onAdd }: WidgetPickerItemsProps) {
  const widget = createWidget(definition.kind, track?.id ?? '', 'idea-timeline-picker');
  const { tag, data, error } = useIdeaHistory(widget, runtime, active);
  const owner = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, track?.branch.id, tag, data, active, disabled]);
  const current = useRef(owner);
  current.current = owner;
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  if (!active) return null;

  const categories = data ? timelineCategories(data.categories) : [];
  const reason = !track ? 'Load a campaign to list its idea categories.'
    : !tag ? 'Choose a country to list this campaign’s idea categories.'
    : error ?? (!data ? 'Loading this campaign’s idea categories…'
      : !categories.length ? 'This campaign’s content has no political-screen idea categories with slots.' : null);
  return <>
    <WidgetPickerItem definition={definition} disabled={disabled} onAdd={onAdd} />
    {reason !== null ? <WidgetPickerItem definition={{ ...definition, title: 'Idea categories' }} disabled reason={reason} onAdd={onAdd} />
      : categories.map(category => {
        const title = category.name ?? category.token;
        return <WidgetPickerItem key={category.token} definition={{ ...definition, title }} disabled={disabled}
          initial={{ title: title || definition.title, options: { category: category.token } }} onAdd={(kind, initial) => {
            if (mounted.current && current.current === owner && active && !disabled) onAdd(kind, initial);
          }} />;
      })}
    {data?.warnings.length ? <p className="field-hint" role="status">{data.warnings.join(' ')}</p> : null}
  </>;
}
