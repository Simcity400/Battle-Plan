import type { WidgetMeta } from '../../dashboard/definitions';
import { columnsControl } from '../../families/settingsControls.ts';
import { LOG_TABLE_BASE, logCountryColumn, logDateColumn, logItemColumn, logKindColumn, logPlaceColumn, logStatusColumn } from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const SERVICE_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ minWidth: 60, maxWidth: 66 }),
  logKindColumn({ minWidth: 34, maxWidth: 40 }),
  logCountryColumn({ minWidth: 46, maxWidth: 76, grow: 1, narrow: 'flag' }),
  logItemColumn({ label: 'Entry', minWidth: 42, grow: 6, narrow: 'primary' }),
  logPlaceColumn({ minWidth: 96, maxWidth: 160, narrow: 'hide', grow: 2, textTone: 'muted' }),
  logStatusColumn({ minWidth: 82, maxWidth: 116, grow: 1 }),
];

export const definition = {
  kind: 'division-service-record', version: 2, title: 'Division service record', icon: 'medal', category: 'Army',
  description: 'The recorded career, awards and removal of a division.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countrySelection: 'settings', countries: { max: Number.MAX_SAFE_INTEGER, all: true }, defaultCountries: { kind: 'selected' },
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 10 },
  availability: { product: 'division-service-record-tail', params: { limit: 1 } },
  options: [
    { key: 'division', section: 'options', label: 'Division', kind: 'content-choice', default: '', automatic: 'Most decorated division', visibleWhen: () => false },
    { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'both',
      choices: [{ value: 'both', label: 'Career and awards' }, { value: 'career', label: 'Career' }, { value: 'awards', label: 'Awards' }] },
    columnsControl(SERVICE_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
  ],
} as const satisfies WidgetMeta;
