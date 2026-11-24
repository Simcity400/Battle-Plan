import { registerWidgets } from '../dashboard/definitions';
import type { WidgetRenderer } from '../dashboard/definitions';
import { widgetMetas } from './definitions';
import { lazyWidget } from './lazyWidget';
import { manifestSlotsForKind, resolveWidgetViewSwitch } from './manifest';
import { useIdeaTimelinePresets } from './idea-timeline/ideaTimelinePresets';
import { useProducedTypeChoices } from './equipment-produced/useProduced';

const extraHooks: Partial<Record<string, Pick<WidgetRenderer, 'usePresets' | 'useOptionChoices'>>> = {
  'idea-timeline': { usePresets: useIdeaTimelinePresets },
  'equipment-produced': { useOptionChoices: useProducedTypeChoices },
};

registerWidgets(widgetMetas.map(meta => {
  const viewSwitch = resolveWidgetViewSwitch(meta.kind, meta);
  return {
    ...meta,
    ...(viewSwitch ? { viewSwitch } : {}),
    ...lazyWidget(meta.kind, manifestSlotsForKind(meta.kind)),
    ...extraHooks[meta.kind],
  };
}));
