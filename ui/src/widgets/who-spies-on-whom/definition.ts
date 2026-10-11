import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'mostCountries', section: 'options', label: 'Countries shown', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 12,
    hint: 'Ranked by operatives sent and received.' },
];
export const definition = {
  kind: 'who-spies-on-whom', version: 1, title: 'Who spies on whom', icon: 'target',
  description: 'Operatives abroad, grouped by country.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'moment', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true }, minSize: { w: 6, h: 7 }, defaultSize: { w: 12, h: 10 }, options: OPTIONS,
} as const satisfies WidgetMeta;
