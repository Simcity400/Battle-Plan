import type { WidgetMeta } from '../../dashboard/definitions';
import { choiceVariants } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION, breakdownVariants } from '../../charts/drill/drillState.ts';
import { TRADE_BREAKDOWN } from './tradeDrill.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'resources', choices: [
    { value: 'resources', label: 'Resource flows' }, { value: 'market', label: 'Market & lend-lease' },
  ] },
  { ...breakdownOption([TRADE_BREAKDOWN], 'Trade flows'), visibleWhen: options => options.view !== 'market' },
  { key: 'tradeSelection', section: 'options', label: 'Selected trade contributors', kind: 'content-choice', automatic: 'Choose a node or ribbon', default: '', visibleWhen: () => false },
  { key: 'tradeExtent', section: 'options', label: 'Contributor history', kind: 'content-choice', automatic: 'The widget’s time range', default: '', visibleWhen: () => false },
  { visibleWhen: options => options.view !== 'market', key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'current',
    choices: [{ value: 'current', label: 'Current (per day)' }, { value: 'total', label: 'Total so far' }, { value: 'average', label: 'Average per trading day' }] },
  { visibleWhen: options => options.view !== 'market', key: 'colour', section: 'options', label: 'Ribbon colour', kind: 'choice', default: 'resource',
    choices: [{ value: 'resource', label: 'By resource' }, { value: 'seller', label: 'By seller' }] },
  { visibleWhen: options => options.view !== 'market', key: 'resources', section: 'options', label: 'Resources', kind: 'content-set',
    automatic: 'All resources', default: '' },
  { visibleWhen: options => options.view !== 'market', key: 'between', section: 'options', label: 'With countries chosen, show', kind: 'choice', default: 'touching',
    choices: [{ value: 'touching', label: 'Their trade with anyone' }, { value: 'between', label: 'Only trade between them' }] },
  { visibleWhen: options => options.view !== 'market', key: 'perSide', section: 'options', label: 'Countries per side', hint: 'The rest are summed into Other.', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 12 },
  { visibleWhen: options => options.view !== 'market', key: 'scale', section: 'options', label: 'Ribbon width', kind: 'choice', default: 'campaign',
    choices: [{ value: 'campaign', label: 'Same scale all campaign' }, { value: 'moment', label: 'Fit the moment shown' }] },
  DRILL_OPTION,
];

export const definition = {
  kind: 'trade-flows', version: 1, title: 'Trade flows', icon: 'flows',
  description: 'Resource flows, market offers and lend-lease agreements.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, ownsOutsideState: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
  variants: [...breakdownVariants('Trade flows', OPTIONS), ...choiceVariants(OPTIONS.find(field => field.key === 'view'), choice => choice.label)],
} as const satisfies WidgetMeta;
