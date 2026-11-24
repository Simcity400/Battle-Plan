import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

export const COLUMNS: readonly TableColumnDefaults[] = [
  { key: 'portrait', label: 'Portrait', as: 'plain', offered: ['plain'] },
  { key: 'country', label: 'Country', as: 'flag', offered: OFFERED.country },
  { key: 'ace', label: 'Ace', as: 'plain', offered: OFFERED.text },
  { key: 'wing', label: 'Air wing', as: 'plain', offered: OFFERED.text },
  { key: 'since', label: 'Ace since', as: 'plain', offered: ['plain'] },
];
export const TABLE_BASE: TableBase = { density: 'comfortable', barsBehind: 'all', overflow: 'tooltip', sort: { column: 'since', direction: 'descending' } };
export const OPTIONS: readonly OptionField[] = tableFamilyOptions(COLUMNS, TABLE_BASE);
export const definition = {
  kind: 'aces', version: 1, title: 'Aces', icon: 'person',
  description: 'Every country’s aces, their recorded wing and aircraft, and when they became aces.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, options: OPTIONS,
} as const satisfies WidgetMeta;
