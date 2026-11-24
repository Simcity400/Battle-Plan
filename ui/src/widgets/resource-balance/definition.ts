import type { WidgetMeta } from '../../dashboard/definitions';
import { choiceVariants } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const BREAKDOWN = { id: 'resource', label: 'Resource breakdown' };

const VIEW_OPTION: OptionField = { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
  choices: [{ value: 'bars', label: 'Balance bars' }, { value: 'time', label: 'Over time' }] };

export const TIME_OPTIONS: readonly OptionField[] = [VIEW_OPTION];

export const OPTIONS: readonly OptionField[] = [
  breakdownOption([BREAKDOWN], 'Resources'), DRILL_OPTION,
  { key: 'resourceCountry', section: 'options', label: 'Breakdown country', kind: 'content-choice', default: '',
    automatic: 'Choose a country and date', visibleWhen: () => false },
  { key: 'partResource', section: 'options', label: 'Breakdown resource', kind: 'content-choice', default: '',
    automatic: 'First resource',
    visibleWhen: options => options.show === 'resource' },
  VIEW_OPTION,
  { key: 'order', section: 'options', label: 'Order', kind: 'choice', default: 'game', visibleWhen: options => options.view === 'bars',
    choices: [{ value: 'game', label: 'Game order' }, { value: 'shortfall', label: 'Biggest shortfall first' }, { value: 'volume', label: 'Largest first' }] },
  { key: 'scale', section: 'options', label: 'Bar length', kind: 'choice', default: 'campaign', visibleWhen: options => options.view === 'bars',
    choices: [{ value: 'campaign', label: 'Same scale all campaign' }, { value: 'moment', label: 'Fit the moment shown' }] },
  { key: 'hideEmpty', section: 'options', label: 'Hide unused resources', kind: 'toggle', default: false,
    visibleWhen: options => options.view === 'bars' },
];

export const definition = {
  kind: 'resource-balance', version: 1, title: 'Resource balance', icon: 'layers',
  description: 'Where a country’s resources come from and where they go, per day, with its trade partners.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 13 },
  countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS,
  variants: choiceVariants(VIEW_OPTION, () => 'Resources over time'),
} as const satisfies WidgetMeta;
