import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TOP_COUNTRIES } from '../../charts/race/options.ts';

export const MEASURES = [{ value: 'industry', label: 'Industry' }, { value: 'fuel', label: 'Fuel' }] as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'industry', choices: [...MEASURES] },
];

export const definition = {
  kind: 'lend-lease-balance', version: 1, title: 'Lend-lease balance', icon: 'chart-bars', category: 'Economy',
  description: 'Lend-lease each country has sent and received.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES, options: OPTIONS, viewSwitch: { option: 'measure' },
} as const satisfies WidgetMeta;
