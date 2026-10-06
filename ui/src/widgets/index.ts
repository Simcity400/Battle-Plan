// The widget registry: every kind in definitions.ts must have a renderer here,
// and TypeScript rejects a missing or extra one.
import { registerWidgets } from '../dashboard/definitions';
import type { WidgetRenderer } from '../dashboard/definitions';
import { widgetMetas } from './definitions';
import agencyComparison from './agency-comparison';
import autonomy from './autonomy';
import commandPower from './command-power';
import casualties from './casualties';
import constructionLog from './construction-log';
import constructionSpending from './construction-spending';
import decryption from './decryption';
import equipmentProduced from './equipment-produced';
import espionageLog from './espionage-log';
import factories from './factories';
import equipmentStockpile from './equipment-stockpile';
import factions from './factions';
import fuel from './fuel';
import experience from './experience';
import stability from './stability';
import ideaTimeline from './idea-timeline';
import map from './map';
import nationalFocus from './national-focus';
import nationalSpirits from './national-spirits';
import nuclearArsenal from './nuclear-arsenal';
import occupiedStates from './occupied-states';
import occupationTrend from './occupation-trend';
import placeholder from './placeholder';
import politicalPower from './political-power';
import productionLines from './production-lines';
import productionSpending from './production-spending';
import research from './research';
import scoreboard from './scoreboard';
import raidLog from './raid-log';
import raidTally from './raid-tally';
import resourceBalance from './resource-balance';
import rulingParty from './ruling-party';
import spiritHistory from './spirit-history';
import specialProjects from './special-projects';
import subjects from './subjects';
import tradeFlows from './trade-flows';
import worldTension from './world-tension';

const renderers: Record<(typeof widgetMetas)[number]['kind'], WidgetRenderer> = { map, 'national-focus': nationalFocus, research, 'political-power': politicalPower, 'command-power': commandPower, stability, 'idea-timeline': ideaTimeline, 'national-spirits': nationalSpirits, 'spirit-history': spiritHistory, 'ruling-party': rulingParty, 'resource-balance': resourceBalance, 'equipment-stockpile': equipmentStockpile, fuel, 'production-lines': productionLines, factories, 'equipment-produced': equipmentProduced, 'production-spending': productionSpending, 'occupied-states': occupiedStates, 'occupation-trend': occupationTrend, experience, factions, subjects, 'trade-flows': tradeFlows, 'world-tension': worldTension, autonomy, 'construction-log': constructionLog, 'construction-spending': constructionSpending, 'special-projects': specialProjects, 'nuclear-arsenal': nuclearArsenal, 'raid-log': raidLog, 'raid-tally': raidTally, decryption, 'espionage-log': espionageLog, 'agency-comparison': agencyComparison, scoreboard, casualties, placeholder };

registerWidgets(widgetMetas.map(meta => ({ ...meta, ...renderers[meta.kind] })));
