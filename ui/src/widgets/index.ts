// The widget registry: every kind in definitions.ts must have a renderer here,
// and TypeScript rejects a missing or extra one.
import { registerWidgets } from '../dashboard/definitions';
import type { WidgetRenderer } from '../dashboard/definitions';
import { widgetMetas } from './definitions';
import commandPower from './command-power';
import ideaTimeline from './idea-timeline';
import map from './map';
import nationalFocus from './national-focus';
import nationalSpirits from './national-spirits';
import placeholder from './placeholder';
import politicalPower from './political-power';
import research from './research';
import spiritHistory from './spirit-history';

const renderers: Record<(typeof widgetMetas)[number]['kind'], WidgetRenderer> = { map, 'national-focus': nationalFocus, research, 'political-power': politicalPower, 'command-power': commandPower, 'idea-timeline': ideaTimeline, 'national-spirits': nationalSpirits, 'spirit-history': spiritHistory, placeholder };

registerWidgets(widgetMetas.map(meta => ({ ...meta, ...renderers[meta.kind] })));
