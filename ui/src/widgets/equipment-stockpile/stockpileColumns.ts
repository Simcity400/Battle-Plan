import type { TableColumnSpec } from '../../families/table/model.ts';
import { OFFERED, type TableBase } from '../../families/table/settings.ts';

export type StockpileLayout = 'sum' | 'parts';

const PICTURE: TableColumnSpec = { key: 'picture', label: 'Picture', as: 'plain', cellKind: 'kind', offered: OFFERED.text, priority: 0, sortable: false, hideable: false, pinned: true };
const NAME: TableColumnSpec = { key: 'equipment', label: 'Equipment', as: 'plain', cellKind: 'item', offered: OFFERED.text, priority: 0, text: true, hideable: false, sortable: false, pinned: true, minWidth: 92, grow: 1 };
const NET: TableColumnSpec = { key: 'net', label: 'Short or over', as: 'plain', cellKind: 'stockpileNet', offered: OFFERED.number, priority: 1, sortable: false, hideable: false, headerGroup: { kind: 'shortOver' }, minWidth: 76, grow: 1 };

function sumColumn(ai: string | null): TableColumnSpec {
  return {
    key: 'sum', label: 'Produced − Need = Balance', unit: 'per day', as: 'plain', cellKind: 'stockpileSum', offered: OFFERED.number, priority: 1, sortable: false, hideable: false, numeric: true, minWidth: 174, grow: 0,
    headerGroup: { kind: 'parts', parts: [{ label: 'Produced' }, { label: 'Need', operator: '−', ...(ai ? { notRecordedReason: ai } : {}) }, { label: 'Balance', operator: '=', ...(ai ? { notRecordedReason: ai } : {}) }] },
  };
}

const BALANCE: TableColumnSpec = { key: 'sum', label: 'Balance a day', as: 'plain', cellKind: 'stockpileBalance', offered: OFFERED.number, priority: 1, sortable: false, hideable: false, numeric: true, minWidth: 174, grow: 0 };

export function stockpileColumns(layout: StockpileLayout, ai: string | null): TableColumnSpec[] {
  const figures = layout === 'parts' ? BALANCE : sumColumn(ai);
  return ai ? [PICTURE, NAME, figures] : [PICTURE, NAME, NET, figures];
}

export const STOCKPILE_BASE: TableBase = { density: 'normal', barsBehind: 'off', showRank: false, sort: { column: 'equipment', direction: 'ascending' } };
