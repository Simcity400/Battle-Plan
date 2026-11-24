import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const DIRECTIONS = [{ value: 'sent', label: 'Sent' }, { value: 'received', label: 'Received' }] as const;
export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'country', label: 'Country', as: 'flag', offered: ['flag'], showNamesDefault: false },
  { key: 'equipment', label: 'Equipment', as: 'plain', offered: OFFERED.text, hideable: false },
  { key: 'monthly', label: 'A month', as: 'faint', offered: OFFERED.number },
  { key: 'delivered', label: 'Delivered', as: 'faint', offered: OFFERED.number },
  { key: 'last', label: 'Last', as: 'plain', offered: OFFERED.text },
  { key: 'since', label: 'Since', as: 'plain', offered: OFFERED.text },
];
export const DEFAULT_SORT = { column: 'delivered', direction: 'descending' } as const;
export const TABLE_BASE: TableBase = { density: 'normal', barsBehind: 'all', showRank: false, sort: DEFAULT_SORT };

export const OPTIONS: readonly OptionField[] = [
  { key: 'direction', section: 'options', label: 'Direction', kind: 'choice', default: 'sent', choices: [...DIRECTIONS] },
  ...tableFamilyOptions(COLUMNS, TABLE_BASE),
];

export const definition = {
  kind: 'lend-lease-agreements', version: 1, title: 'Lend-lease agreements', icon: 'link', category: 'Economy',
  description: 'Lend-lease each country sends or receives: equipment, monthly amount and delivered.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS, viewSwitch: { option: 'direction' },
} as const satisfies WidgetMeta;
