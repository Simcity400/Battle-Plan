import type { WidgetMeta } from '../../dashboard/definitions';
import { columnsControl } from '../../families/settingsControls.ts';
import { LOG_TABLE_BASE, logActionColumn, logCountryColumn, logDateColumn, logItemColumn, logKindColumn, logPairColumn, logStatusColumn } from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const NAVAL_BATTLES_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ sortable: true, minWidth: 70, maxWidth: 70 }),
  logKindColumn({ minWidth: 52, maxWidth: 52 }),
  logCountryColumn({ label: '', minWidth: 68, grow: 0 }),
  logItemColumn({ label: 'Battle', minWidth: 42, narrow: 'primary' }),
  { ...logPairColumn('shipsSunk', 'Ships sunk', 'total', 68), minWidth: 88, grow: 0, partialReason: 'Sinkings seen during the battle: a lower bound until its result is recorded' },
  { ...logPairColumn('icLost', 'IC lost', 'hide', 57), grow: 0, partialReason: 'IC of the sinkings seen during the battle: a lower bound until its result is recorded' },
  { ...logPairColumn('ships', 'Ships', 'hide', 57), grow: 0 },
  logStatusColumn({ label: 'Result', widest: ['Defender won'], minWidth: 136, maxWidth: 136, narrow: 'hide' }),
  logActionColumn({ minWidth: 84, narrow: 'hide' }),
];

export const definition = {
  kind: 'naval-battles', version: 2, title: 'Naval battles', icon: 'anchor', category: 'Navy',
  description: 'Naval battles, their participants, ships sunk and results.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'all' },
  options: [columnsControl(NAVAL_BATTLES_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY })],
} as const satisfies WidgetMeta;
