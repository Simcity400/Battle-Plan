import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'flow',
    choices: [{ value: 'flow', label: 'Flow' }, { value: 'table', label: 'Table' }, { value: 'matrix', label: 'Matrix' }] },
  { key: 'between', section: 'options', label: 'With countries chosen, show', kind: 'choice', default: 'touching',
    choices: [{ value: 'touching', label: 'Their trade with anyone' }, { value: 'between', label: 'Only trade between them' }] },
];

export const definition = {
  kind: 'trade-flows', version: 2, title: 'Trade flows', icon: 'flows',
  description: 'What a country buys and sells, with whom and how much a day.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
} as const satisfies WidgetMeta;
