import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'order', section: 'options', label: 'Factions', kind: 'choice', default: 'founded',
    choices: [{ value: 'founded', label: 'Date founded' }, { value: 'members', label: 'Most members' }, { value: 'name', label: 'Name' }] },
  { key: 'members', section: 'options', label: 'Members', kind: 'choice', default: 'joined',
    choices: [{ value: 'joined', label: 'Date joined' }, { value: 'name', label: 'Name' }] },
  { key: 'ended', section: 'options', label: 'Dissolved factions and former members', kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
];

export const definition = {
  kind: 'factions', version: 1, title: 'Factions', icon: 'shield',
  description: 'Every faction and its members over time, with each faction’s leader marked.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  factionColours: true,
  options: OPTIONS,
} as const satisfies WidgetMeta;
