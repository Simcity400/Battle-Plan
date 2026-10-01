// Pure widget metadata for node tests and the layout model; no React or CSS.
import { registerWidgets } from '../dashboard/definitions.ts';
import { definition as commandPower } from './command-power/definition.ts';
import { definition as fuel } from './fuel/definition.ts';
import { definition as experience } from './experience/definition.ts';
import { definition as ideaTimeline } from './idea-timeline/definition.ts';
import { definition as map } from './map/definition.ts';
import { definition as nationalFocus } from './national-focus/definition.ts';
import { definition as nationalSpirits } from './national-spirits/definition.ts';
import { definition as occupiedStates } from './occupied-states/definition.ts';
import { definition as placeholder } from './placeholder/definition.ts';
import { definition as politicalPower } from './political-power/definition.ts';
import { definition as research } from './research/definition.ts';
import { definition as resourceBalance } from './resource-balance/definition.ts';
import { definition as rulingParty } from './ruling-party/definition.ts';
import { definition as spiritHistory } from './spirit-history/definition.ts';
import { definition as stability } from './stability/definition.ts';

export const widgetMetas = [map, nationalFocus, research, politicalPower, commandPower, stability, ideaTimeline, nationalSpirits, spiritHistory, rulingParty, resourceBalance, fuel, occupiedStates, experience, placeholder] as const;

registerWidgets(widgetMetas);
