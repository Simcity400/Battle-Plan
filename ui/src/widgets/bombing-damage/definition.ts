import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TOP_COUNTRIES } from '../../charts/race/options.ts';

export const OPTIONS: readonly OptionField[] = [];

export const definition = {
  kind: 'bombing-damage', version: 1, title: 'Bombing damage', icon: 'target',
  description: 'Applied aircraft bombing damage inflicted and suffered, in building levels.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true }, defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
} as const satisfies WidgetMeta;
