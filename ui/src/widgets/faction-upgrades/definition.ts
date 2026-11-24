import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { FACTION_OPTION } from '../../dashboard/factionOption.ts';

export const OPTIONS: readonly OptionField[] = [
  FACTION_OPTION,
  { key: 'endedProgrammes', section: 'options', label: 'Ended programmes', kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
];

export const definition = {
  kind: 'faction-upgrades', version: 1, title: 'Faction upgrades & rules', icon: 'history', category: 'Diplomacy',
  description: 'A faction’s rules, programmes, upgrades and member minimums over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 8 }, countries: { max: 1 }, options: OPTIONS, factionColours: true,
} as const satisfies WidgetMeta;
