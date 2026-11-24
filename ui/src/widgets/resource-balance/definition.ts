import type { WidgetMeta } from '../../dashboard/definitions';
import { choiceVariants } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';

const VIEW_OPTION: OptionField = { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
  choices: [{ value: 'bars', label: 'Bars' }, { value: 'time', label: 'Time' }] };

export const TIME_OPTIONS: readonly OptionField[] = [VIEW_OPTION];

export const OPTIONS: readonly OptionField[] = [
  VIEW_OPTION,
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
