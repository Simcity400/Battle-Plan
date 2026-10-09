import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'direction', section: 'options', label: 'Damage', kind: 'choice', default: 'taken',
    choices: [{ value: 'taken', label: 'Suffered' }, { value: 'dealt', label: 'Inflicted' }] },
  levelOption('buildingLevel', 'Building type', 'The recorded building spelling, resolved through this campaign’s content stack'),
  levelOption('stateLevel', 'State', 'The recorded target state, named at the shown read'),
  ...SUNBURST_OPTIONS,
];

export const definition = {
  kind: 'bombing-damage-breakdown', version: 1, title: 'Bombing damage breakdown', icon: 'sunburst',
  description: 'One country’s applied bombing damage, by building type and state.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  availability: { product: 'military-air-ledger', params: { family: 'bombing' } }, options: OPTIONS,
} as const satisfies WidgetMeta;
