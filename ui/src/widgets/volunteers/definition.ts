import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'between', section: 'options', label: 'With countries chosen, show', kind: 'choice', default: 'touching',
    choices: [{ value: 'touching', label: 'Their forces with anyone' }, { value: 'between', label: 'Only forces between them' }] },
];

export const definition = {
  kind: 'volunteers', version: 2, title: 'Volunteers & expeditionaries', icon: 'flows', category: 'Diplomacy',
  description: 'Volunteer and expeditionary divisions from each sender to each host.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS,
} as const satisfies WidgetMeta;
