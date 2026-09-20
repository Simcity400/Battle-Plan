import type { WidgetSpec, WidgetTime } from './model';

/** Code-owned capabilities. Renderers live in widgets.tsx; the scheduler uses
 * these data/time contracts and never switches on a widget's type. */
export interface WidgetDefinition {
  kind: string;
  version: number;
  title: string;
  timeModes: readonly WidgetTime['mode'][];
  defaultTime: WidgetTime['mode'] & ('follow' | 'campaign');
  scope: 'moment' | 'history';
  data: 'native' | 'countries' | 'none';
  minSize: { w: number; h: number };
  defaultSize: { w: number; h: number };
}

const registered: WidgetDefinition[] = [
  { kind: 'map', version: 1, title: 'Campaign map', timeModes: ['campaign', 'follow', 'pin'],
    defaultTime: 'follow', scope: 'moment', data: 'native', minSize: { w: 3, h: 8 }, defaultSize: { w: 7, h: 11 } },
  { kind: 'placeholder', version: 1, title: 'Untitled panel', timeModes: ['campaign', 'follow'],
    defaultTime: 'campaign', scope: 'moment', data: 'none', minSize: { w: 2, h: 3 }, defaultSize: { w: 4, h: 6 } },
];
export const widgetDefinitions: readonly WidgetDefinition[] = registered;
const definitions = new Map(registered.map((definition) => [definition.kind, definition]));
/** Register code-defined data products before mounting the dashboard. */
export function registerWidgetDefinition(definition: WidgetDefinition): void {
  if (definitions.has(definition.kind)) throw new Error(`Widget type already registered: ${definition.kind}`);
  definitions.set(definition.kind, definition);
  registered.push(definition);
}
export function widgetDefinition(kind: string): WidgetDefinition | undefined { return definitions.get(kind); }

export function widgetUnavailable(widget: WidgetSpec): string | null {
  const definition = widgetDefinition(widget.kind);
  if (!definition) return `Widget type “${widget.kind}” is not installed. Its saved settings are retained.`;
  if (widget.configVersion !== definition.version) return `Widget settings version ${widget.configVersion} is unsupported. Its saved settings are retained.`;
  if (!definition.timeModes.includes(widget.time.mode)) return 'This widget does not support that time mode. Choose a supported mode in its settings.';
  return null;
}
