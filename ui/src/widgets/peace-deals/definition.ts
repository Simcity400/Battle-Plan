import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'actions', section: 'options', label: 'Peace actions', hint: 'Show conferences with these actions.',
    kind: 'content-set', automatic: 'Every action', default: '' },
  ...LOG_OPTIONS,
];

export const definition = {
  kind: 'peace-deals', version: 1, title: 'Peace deals', icon: 'history', category: 'Diplomacy',
  description: 'Peace conferences, winners and outcomes.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countrySelection: false,
  countries: { max: Number.MAX_SAFE_INTEGER },
  defaultCountries: { kind: 'all' }, minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 12 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
