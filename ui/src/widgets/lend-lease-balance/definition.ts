import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { TOP_COUNTRIES } from '../../charts/race/options.ts';

export const PARTNERS = { id: 'partners', label: 'Partners' };
export const MEASURES = [{ value: 'industry', label: 'Industry' }, { value: 'fuel', label: 'Fuel' }] as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'industry', choices: [...MEASURES] },
  { key: 'values', section: 'options', label: 'Values at bar ends', kind: 'toggle', default: true },
  DRILL_OPTION,
];

export const definition = {
  kind: 'lend-lease-balance', version: 1, title: 'Lend-lease balance', icon: 'chart-bars', category: 'Economy',
  description: 'Lend-lease each country has sent and received, and with whom.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES, options: OPTIONS, viewSwitch: { option: 'measure' },
} as const satisfies WidgetMeta;
