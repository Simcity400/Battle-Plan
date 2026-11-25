import type { WidgetMeta } from '../../dashboard/definitions';
import { columnsControl } from '../../families/settingsControls.ts';
import { LOG_TABLE_BASE, logActionColumn, logCountryColumn, logDateColumn, logItemColumn, logKindColumn, logPlaceColumn, logStatusColumn } from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { OFFERED, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

function sides(key: string, label: string, slot: number, narrow: 'hide' | 'total'): TableColumnSpec {
  return { key, label, priority: 1, as: 'plain', numeric: true, format: 'integer', offered: OFFERED.text,
    headerGroup: { kind: 'pair', labels: ['Attacker', 'Defender'], slot }, minWidth: 82, grow: 0, narrow };
}

export const LAND_BATTLES_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ sortable: true, minWidth: 70, maxWidth: 70 }),
  logKindColumn({ minWidth: 52, maxWidth: 52 }),
  logCountryColumn({ label: '', minWidth: 68, grow: 0 }),
  logItemColumn({ label: 'Battle', minWidth: 42, narrow: 'primary' }),
  logPlaceColumn({ label: 'Province', minWidth: 140, maxWidth: 140, grow: 0, narrow: 'hide', textTone: 'muted' }),
  sides('casualties', 'Casualties', 68, 'total'),
  sides('icLost', 'IC lost', 57, 'hide'),
  sides('divisions', 'Divisions', 58, 'hide'),
  logStatusColumn({ label: 'Result', widest: ['Defender won'], minWidth: 136, maxWidth: 136, narrow: 'hide' }),
  logActionColumn({ minWidth: 84, narrow: 'hide' }),
];

export const definition = {
  kind: 'land-battles', version: 2, title: 'Land battles', icon: 'swords', category: 'Army',
  description: 'Land battles, their participants, casualties, equipment losses and results.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'all' },
  availability: { product: 'land-battles-tail', params: { limit: 1 } },
  options: [columnsControl(LAND_BATTLES_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY })],
} as const satisfies WidgetMeta;
