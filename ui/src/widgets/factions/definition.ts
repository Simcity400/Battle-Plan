import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'order', section: 'options', label: 'Factions', kind: 'choice', default: 'founded',
    choices: [{ value: 'founded', label: 'Date founded' }, { value: 'members', label: 'Most members' }, { value: 'name', label: 'Name' }] },
  { key: 'members', section: 'options', label: 'Members', kind: 'choice', default: 'joined',
    choices: [{ value: 'joined', label: 'Date joined' }, { value: 'name', label: 'Name' }] },
  { key: 'ended', section: 'options', label: 'Dissolved factions and former members', kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
  { key: 'legend', section: 'options', label: 'Faction colours above the timeline', hint: 'Click a swatch to change a faction’s colour',
    kind: 'toggle', default: true },
  { key: 'colourScope', section: 'options', label: 'A colour changed from a swatch applies to', kind: 'choice', default: 'all',
    choices: [{ value: 'all', label: 'All campaigns' }, { value: 'campaign', label: 'This campaign only' }],
    visibleWhen: options => options.legend === true },
];

export const definition = {
  kind: 'factions', version: 1, title: 'Factions', icon: 'shield',
  description: 'Every faction and its members over time, with each faction’s leader marked.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 10 },
  countries: { max: 64, all: true },
  options: OPTIONS,
} as const satisfies WidgetMeta;
