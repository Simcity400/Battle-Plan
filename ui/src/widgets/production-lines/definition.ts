import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { DRILL_OPTION, breakdownOption } from '../../charts/drill/drillState.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'productionCountry', section: 'options', label: 'Selected country', kind: 'content-choice', automatic: 'Selected country', default: '', visibleWhen: () => false },
  breakdownOption([{ id: 'efficiency', label: 'Efficiency breakdown' }, { id: 'output', label: 'Output breakdown' }], 'Production lines'),
  { key: 'line', section: 'options', label: 'Production line', kind: 'content-choice', automatic: 'Choose a production line', default: '' },
  { key: 'factor', section: 'options', label: 'Production factor', kind: 'content-choice', automatic: 'Default breakdown', default: '' },
  DRILL_OPTION,
  { key: 'order', section: 'options', label: 'Order', hint: 'Live table', kind: 'choice', default: 'game',
    choices: [{ value: 'game', label: 'Game order' }, { value: 'factories', label: 'Factories' }, { value: 'output', label: 'Output' }] },
  { key: 'columns', section: 'options', label: 'Columns', hint: 'Live table', kind: 'choice', default: 'basic',
    choices: [{ value: 'basic', label: 'Basic' }, { value: 'more', label: '+ Priority, IC per unit and MIO' }] },
  { key: 'ended', section: 'options', label: 'Ended lines', hint: 'History', kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
];

export const definition = {
  kind: 'production-lines', version: 1, title: 'Production lines', icon: 'grid',
  description: 'A country’s production lines: factories, efficiency, output and totals, live or across the campaign.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 5, h: 6 }, defaultSize: { w: 12, h: 14 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
