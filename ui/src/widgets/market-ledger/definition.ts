import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { columnsControl } from '../../families/settingsControls.ts';
import {
  LOG_TABLE_BASE,
  LOG_SEARCH_OPTION,
  logActionColumn,
  logCountryColumn,
  logDateColumn,
  logItemColumn,
  logKindColumn,
  logStatusColumn,
} from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { OFFERED, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const MARKET_LEDGER_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ sortable: true }),
  logKindColumn({ minWidth: 34, maxWidth: 42 }),
  logCountryColumn({ sortable: true }),
  logItemColumn({ sortable: true, label: 'Equipment', minWidth: 180 }),
  {
    key: 'delivered',
    label: 'Delivered',
    as: 'plain',
    offered: OFFERED.text,
    priority: 1,
    sortable: true,
    numeric: true,
    minWidth: 112,
    widest: ['999 / 999 units'],
  },
  {
    key: 'paid',
    label: 'Paid',
    unit: 'CIC',
    as: 'plain',
    offered: OFFERED.text,
    priority: 2,
    sortable: true,
    numeric: true,
    minWidth: 108,
    widest: ['−9,999.9 CIC'],
  },
  logStatusColumn({ sortable: true, minWidth: 136, widest: ['Contract started'] }),
  logActionColumn({ minWidth: 26, maxWidth: 32 }),
];

export const OPTIONS: readonly OptionField[] = [
  LOG_SEARCH_OPTION,
  { key: 'showRequests', section: 'options', label: 'Include open requests', kind: 'toggle', default: true },
  { key: 'showEnded', section: 'options', label: 'Include ended contracts', kind: 'toggle', default: true },
  columnsControl(MARKET_LEDGER_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
];

export const definition = {
  kind: 'market-ledger',
  version: 2,
  title: 'Market ledger',
  icon: 'history',
  category: 'Economy',
  description: 'One row per market contract with delivered units and CIC paid, expanding to dated deliveries and payments.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 4, h: 6 },
  defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'selected' },
  options: OPTIONS,
} as const satisfies WidgetMeta;
