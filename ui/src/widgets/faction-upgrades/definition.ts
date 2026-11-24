import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [];

export const definition = {
  kind: 'faction-upgrades', version: 1, title: 'Faction upgrades & rules', icon: 'history', category: 'Diplomacy',
  description: 'The rules and upgrades in force in a country’s faction, with the date each began.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 8 }, countries: { max: 1 }, options: OPTIONS, factionColours: true,
} as const satisfies WidgetMeta;
