import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const KINDS = [{ value: 'land', label: 'Volunteers' }, { value: 'expeditionary', label: 'Expeditionary' }] as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'kind', section: 'options', label: 'Kind', kind: 'choice', default: 'land', choices: KINDS },
  { key: 'between', section: 'options', label: 'With countries chosen, show', kind: 'choice', default: 'touching',
    choices: [{ value: 'touching', label: 'Their forces with anyone' }, { value: 'between', label: 'Only forces between them' }] },
  { key: 'perSide', section: 'options', label: 'Countries per side', hint: 'The rest are summed into Other.', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 12 },
  { key: 'scale', section: 'options', label: 'Ribbon width', kind: 'choice', default: 'campaign',
    choices: [{ value: 'campaign', label: 'Same scale all campaign' }, { value: 'moment', label: 'Fit the moment shown' }] },
  DRILL_OPTION,
];

export const definition = {
  kind: 'volunteers', version: 1, title: 'Volunteers & expeditionaries', icon: 'flows', category: 'Diplomacy',
  description: 'Volunteer and expeditionary forces from each sender to each host.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS, viewSwitch: { option: 'kind' },
} as const satisfies WidgetMeta;
