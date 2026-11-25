import type { WidgetMeta } from '../../dashboard/definitions';
import { columnsControl } from '../../families/settingsControls.ts';
import { LOG_TABLE_BASE, logActionColumn, logCountryColumn, logDateColumn, logItemColumn, logKindColumn } from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const PEACE_DEALS_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ minWidth: 68 }),
  logKindColumn({ minWidth: 36 }),
  logCountryColumn({ label: 'Sides', minWidth: 60, grow: 0 }),
  logItemColumn({ key: 'terms', label: 'Terms', minWidth: 80, grow: 9, narrow: 'primary', textTone: 'muted', widest: ['White peace'] }),
  logActionColumn({ minWidth: 24, narrow: 'hide' }),
];

export const definition = {
  kind: 'peace-deals', version: 2, title: 'Peace deals', icon: 'olive-branch', category: 'Diplomacy',
  description: 'Peace conferences, winners and outcomes.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countrySelection: 'settings', countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'all' },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 12 },
  options: [columnsControl(PEACE_DEALS_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY })],
} as const satisfies WidgetMeta;
