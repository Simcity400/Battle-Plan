import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';
import { STATUS_CHOICES } from './decisionsData.ts';

export const OPTIONS: readonly OptionField[] = [
  ...STATUS_CHOICES.map(status => ({ key: status.value, section: 'options' as const, label: status.label,
    ...('hint' in status ? { hint: status.hint } : {}), kind: 'toggle' as const, default: true })),
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  ...LOG_OPTIONS,
];

export const definition = {
  kind: 'decisions', version: 1, title: 'Decisions', icon: 'ballot', category: 'Politics',
  description: 'Decisions and missions each country took, what they cost and how they ended.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'selected' },
  options: OPTIONS,
} as const satisfies WidgetMeta;
