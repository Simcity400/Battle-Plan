import { registerWidgets } from '../dashboard/definitions';
import { widgetMetas } from './definitions';
import { lazyWidget } from './lazyWidget';
import { manifestSlotsForKind, resolveWidgetViewSwitch } from './manifest';

registerWidgets(widgetMetas.map(meta => {
  const viewSwitch = resolveWidgetViewSwitch(meta.kind, meta);
  return {
    ...meta,
    ...(viewSwitch ? { viewSwitch } : {}),
    ...lazyWidget(meta.kind, manifestSlotsForKind(meta.kind)),
  };
}));
