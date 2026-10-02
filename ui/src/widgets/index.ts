// The widget registry: every kind in definitions.ts must have a renderer here,
// and TypeScript rejects a missing or extra one.
import { registerWidgets } from '../dashboard/definitions';
import type { WidgetRenderer } from '../dashboard/definitions';
import { widgetMetas } from './definitions';
import commandPower from './command-power';
import equipmentStockpile from './equipment-stockpile';
import factions from './factions';
import fuel from './fuel';
import experience from './experience';
import stability from './stability';
import ideaTimeline from './idea-timeline';
import map from './map';
import nationalFocus from './national-focus';
import nationalSpirits from './national-spirits';
import occupiedStates from './occupied-states';
import placeholder from './placeholder';
import politicalPower from './political-power';
import research from './research';
import resourceBalance from './resource-balance';
import rulingParty from './ruling-party';
import spiritHistory from './spirit-history';
import subjects from './subjects';
import tradeFlows from './trade-flows';

const renderers: Record<(typeof widgetMetas)[number]['kind'], WidgetRenderer> = { map, 'national-focus': nationalFocus, research, 'political-power': politicalPower, 'command-power': commandPower, stability, 'idea-timeline': ideaTimeline, 'national-spirits': nationalSpirits, 'spirit-history': spiritHistory, 'ruling-party': rulingParty, 'resource-balance': resourceBalance, 'equipment-stockpile': equipmentStockpile, fuel, 'occupied-states': occupiedStates, experience, factions, subjects, 'trade-flows': tradeFlows, placeholder };

registerWidgets(widgetMetas.map(meta => ({ ...meta, ...renderers[meta.kind] })));
