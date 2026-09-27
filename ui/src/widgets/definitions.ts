// Pure widget metadata for node tests and the layout model; no React or CSS.
import { registerWidgets } from '../dashboard/definitions.ts';
import { definition as commandPower } from './command-power/definition.ts';
import { definition as map } from './map/definition.ts';
import { definition as nationalFocus } from './national-focus/definition.ts';
import { definition as placeholder } from './placeholder/definition.ts';
import { definition as politicalPower } from './political-power/definition.ts';
import { definition as research } from './research/definition.ts';

export const widgetMetas = [map, nationalFocus, research, politicalPower, commandPower, placeholder] as const;

registerWidgets(widgetMetas);
