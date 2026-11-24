import type { RenderedWidget } from '../dashboard/definitions';
import type { WidgetSpec } from '../dashboard/model';
import { manifestFamilyView, resolveWidgetActiveView, widgetViewOptions, type ResolvedWidgetOptions } from './manifest.ts';

const rendered = new WeakMap<WidgetSpec, RenderedWidget>();

function withoutStored(widget: WidgetSpec): RenderedWidget {
  let held = rendered.get(widget);
  if (!held) {
    const { options: _stored, ...rest } = widget;
    rendered.set(widget, held = rest);
  }
  return held;
}

export function viewSlotProps<P extends { readonly widget?: WidgetSpec }>(kind: string, props: P): Omit<P, 'widget'> & {
  readonly widget?: WidgetSpec | RenderedWidget;
  readonly options: ResolvedWidgetOptions;
  readonly view: string;
} {
  const stored = props.widget?.options;
  const view = resolveWidgetActiveView(kind, stored);
  return {
    ...props,
    ...(props.widget ? { widget: manifestFamilyView(kind, view) ? withoutStored(props.widget) : props.widget } : {}),
    options: widgetViewOptions(kind, stored, view),
    view,
  };
}
