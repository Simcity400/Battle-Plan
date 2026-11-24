import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TOP_COUNTRIES } from '../../charts/race/options.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'order', section: 'options', label: 'Order', kind: 'choice', default: 'faction',
    choices: [{ value: 'faction', label: 'Faction' }, { value: 'name', label: 'Name' }] },
];

export const definition = {
  kind: 'diplomatic-relations', version: 1, title: 'Diplomatic relations', icon: 'handshake', category: 'Diplomacy',
  description: 'Every chosen country’s opinion of every other.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 9 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
} as const satisfies WidgetMeta;
