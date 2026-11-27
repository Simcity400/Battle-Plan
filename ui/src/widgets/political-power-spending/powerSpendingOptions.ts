import type { OptionField } from '../../dashboard/options';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { OFFERED, tableFamilyOptions, type TableBase } from '../../families/table/settings.ts';
import type { SpendCurrency } from '../../generated/dashboard';

const UNIT: Readonly<Record<SpendCurrency, string>> = { Political: 'PP', Command: 'CP' };

export function powerSpendingColumns(currency: SpendCurrency): TableColumnSpec[] {
  return [
    { key: 'picture', label: '', as: 'plain', cellKind: 'kind', offered: OFFERED.text, priority: 0, sortable: false, hideable: false, pinned: true },
    { key: 'item', label: 'Spent on', as: 'plain', cellKind: 'item', offered: OFFERED.text, priority: 0, text: true, hideable: false, pinned: true, widest: ['Organisation upkeep'] },
    { key: 'spent', label: 'Spent', unit: UNIT[currency], as: 'faint', barEligible: true, offered: OFFERED.number, priority: 1, numeric: true, widest: ['99,999'] },
    { key: 'share', label: 'Share', as: 'plain', format: 'share', barEligible: false, offered: OFFERED.number, priority: 2, numeric: true, widest: ['100%', '<0.1%'] },
    { key: 'last', label: 'Last', as: 'plain', cellKind: 'date', offered: OFFERED.number, priority: 3, numeric: true, widest: ['28 Aug'] },
  ];
}

export const POWER_SPENDING_BASE: TableBase = { density: 'normal', barsBehind: 'all', showRank: false, sort: { column: 'spent', direction: 'descending' } };

export function powerSpendingOptions(currency: SpendCurrency): readonly OptionField[] {
  return tableFamilyOptions(powerSpendingColumns(currency), POWER_SPENDING_BASE);
}
