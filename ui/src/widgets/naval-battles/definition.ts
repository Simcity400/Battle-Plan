import type { WidgetMeta } from '../../dashboard/definitions';
import { columnsControl } from '../../families/settingsControls.ts';
import { LOG_TABLE_BASE, logCountryColumn, logDateColumn, logItemColumn, logKindColumn, logPlaceColumn, logPairColumn, logStatusColumn } from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const NAVAL_BATTLES_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ sortable: true, minWidth: 84, maxWidth: 90, widest: ['by 30 Aug'] }),
  logKindColumn({ minWidth: 36 }),
  logCountryColumn({ label: '', minWidth: 55, grow: 0 }),
  logItemColumn({ label: 'Battle', minWidth: 42, narrow: 'primary', grow: 6 }),
  logPlaceColumn({ label: 'Ocean', minWidth: 134, maxWidth: 134, grow: 0, narrow: 'hide', textTone: 'muted' }),
  logPairColumn('shipsSunk', 'Ships sunk', 'total'),
  logPairColumn('icLost', 'IC lost', 'hide'),
  logPairColumn('ships', 'Ships', 'hide'),
  logStatusColumn({ label: 'Result', widest: ['Defender won'], narrow: 'hide' }),
];

export const definition = {
  kind: 'naval-battles', version: 2, title: 'Naval battles', icon: 'anchor', category: 'Navy',
  description: 'Naval battles, their participants, ships sunk and results.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'all' },
  availability: { product: 'naval-battles-tail', params: { limit: 1 } },
  options: [columnsControl(NAVAL_BATTLES_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY })],
} as const satisfies WidgetMeta;
