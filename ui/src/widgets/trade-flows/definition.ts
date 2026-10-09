import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { TRADE_BREAKDOWN } from './tradeDrill.ts';

export const OPTIONS: readonly OptionField[] = [
  breakdownOption([TRADE_BREAKDOWN], 'Trade flows'),
  { key: 'tradeSelection', section: 'options', label: 'Selected trade contributors', kind: 'content-choice', automatic: 'Choose a node or ribbon', default: '', visibleWhen: () => false },
  { key: 'tradeExtent', section: 'options', label: 'Contributor history', kind: 'content-choice', automatic: 'The widget’s time range', default: '', visibleWhen: () => false },
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'current',
    choices: [{ value: 'current', label: 'Current (per day)' }, { value: 'total', label: 'Total so far' }, { value: 'average', label: 'Campaign average (per day)' }] },
  { key: 'colour', section: 'options', label: 'Ribbon colour', kind: 'choice', default: 'resource',
    choices: [{ value: 'resource', label: 'By resource' }, { value: 'seller', label: 'By seller' }] },
  { key: 'resources', section: 'options', label: 'Resources', hint: 'From the campaign’s own content', kind: 'content-set',
    automatic: 'All resources', default: '' },
  { key: 'between', section: 'options', label: 'With countries chosen, show', kind: 'choice', default: 'touching',
    choices: [{ value: 'touching', label: 'Their trade with anyone' }, { value: 'between', label: 'Only trade between them' }] },
  { key: 'perSide', section: 'options', label: 'Countries per side', hint: 'The rest are summed into one', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 12 },
  { key: 'scale', section: 'options', label: 'Ribbon width', kind: 'choice', default: 'campaign',
    choices: [{ value: 'campaign', label: 'Same scale all campaign' }, { value: 'moment', label: 'Fit the moment shown' }] },
  DRILL_OPTION,
];

export const definition = {
  kind: 'trade-flows', version: 1, title: 'Trade flows', icon: 'flows',
  description: 'Who sells which resources to whom, as ribbons from sellers to buyers that grow and shrink with the trade.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
} as const satisfies WidgetMeta;
