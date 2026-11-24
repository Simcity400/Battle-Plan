import { TOP_COUNTRIES } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'butterfly',
    choices: [{ value: 'butterfly', label: 'Butterfly' }, { value: 'race', label: 'Race' }] },
  { key: 'includeUnknown', section: 'options', label: 'Include casualties from unknown attackers', kind: 'toggle', default: false },
];

export const definition = {
  kind: 'casualties', version: 2, title: 'Casualties', icon: 'target',
  description: 'Each country’s casualties taken and dealt.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
} as const satisfies WidgetMeta;
