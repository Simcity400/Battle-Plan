import { TOP_COUNTRIES } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'Views', kind: 'choice', default: 'butterfly',
    choices: [{ value: 'butterfly', label: 'Dealt vs taken' }, { value: 'race', label: 'Race of losses taken' }] },
  { key: 'includeUnknown', section: 'options', label: 'Include losses from unknown attackers', kind: 'toggle', default: false },
  { key: 'values', section: 'options', label: 'Values at bar ends', kind: 'toggle', default: true, visibleWhen: options => options.view !== 'race' },
  { key: 'ratio', section: 'options', label: 'Ratio column', kind: 'toggle', default: true, visibleWhen: options => options.view !== 'race' },
];
export const definition = {
  kind: 'casualties', version: 1, title: 'Casualties', icon: 'target',
  description: 'Each country’s casualties dealt and taken, with their ratio.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
} as const satisfies WidgetMeta;
