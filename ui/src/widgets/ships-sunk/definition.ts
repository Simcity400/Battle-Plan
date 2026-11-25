import type { WidgetMeta } from '../../dashboard/definitions';
import { columnsControl } from '../../families/settingsControls.ts';
import { LOG_TABLE_BASE, logCountryColumn, logDateColumn, logItemColumn, logKindColumn, logPlaceColumn } from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const SHIPS_SUNK_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ minWidth: 70, maxWidth: 70 }),
  logKindColumn({ label: 'Class', minWidth: 52, maxWidth: 52 }),
  logCountryColumn({ label: 'Killer and victim', minWidth: 68, grow: 0 }),
  logItemColumn({ label: 'Ship', minWidth: 48, maxWidth: 48, narrow: 'primary', grow: 2.4 }),
  logPlaceColumn({ label: 'Sea zone', minWidth: 210, maxWidth: 210, grow: 0, narrow: 'hide', textTone: 'muted' }),
  logPlaceColumn({ key: 'cause', label: 'Cause', minWidth: 270, maxWidth: 270, narrowWidth: 92, grow: 1, textTone: 'secondary' }),
];

export const definition = {
  kind: 'ships-sunk', version: 2, title: 'Ships sunk', icon: 'ship', category: 'Navy',
  description: 'Ships sunk, with their owners, credited countries, classes and sea zones.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'all' },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 },
  availability: { product: 'ships-sunk-tail', params: { limit: 1 } },
  options: [
    { key: 'classes', section: 'options', label: 'Ship classes', kind: 'content-set', default: '', automatic: 'All classes', visibleWhen: () => false },
    columnsControl(SHIPS_SUNK_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
  ],
} as const satisfies WidgetMeta;
