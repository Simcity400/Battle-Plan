// The widget registry: every kind in definitions.ts must have a renderer here,
// and TypeScript rejects a missing or extra one.
import { registerWidgets } from '../dashboard/definitions';
import type { WidgetRenderer } from '../dashboard/definitions';
import { widgetMetas } from './definitions';
import map from './map';
import nationalFocus from './national-focus';
import placeholder from './placeholder';
import research from './research';

const renderers: Record<(typeof widgetMetas)[number]['kind'], WidgetRenderer> = { map, 'national-focus': nationalFocus, research, placeholder };

registerWidgets(widgetMetas.map(meta => ({ ...meta, ...renderers[meta.kind] })));
