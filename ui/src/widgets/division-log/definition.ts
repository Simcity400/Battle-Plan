import type { WidgetMeta } from '../../dashboard/definitions';
import { columnsControl } from '../../families/settingsControls.ts';
import { LOG_TABLE_BASE, logActionColumn, logCountryColumn, logDateColumn, logItemColumn, logKindColumn, logPlaceColumn } from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const DIVISION_LOG_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ minWidth: 60 }),
  logKindColumn({ minWidth: 36 }),
  logItemColumn({ label: 'Division', minWidth: 94, narrow: 'primary', grow: 6 }),
  logCountryColumn({ as: 'flagName', showNamesDefault: true, minWidth: 36, maxWidth: 190, narrow: 'flag', textTone: 'secondary', grow: 1.8 }),
  logPlaceColumn({ narrow: 'hide', textTone: 'muted', minWidth: 110, maxWidth: 170, grow: 1.5 }),
  logCountryColumn({ key: 'outcome', label: 'Removal', as: 'flagName', showNamesDefault: true, minWidth: 48, maxWidth: 160, narrow: 'flag', textTone: 'secondary', grow: 1.5 }),
  logActionColumn({ minWidth: 20, maxWidth: 24 }),
];

export const definition = {
  kind: 'division-log', version: 2, title: 'Division log', icon: 'shield',
  description: 'Division creations, losses and transfers, with templates, locations and attackers.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: { product: 'division-log-tail', params: { limit: 1 } },
  options: [
    { key: 'events', section: 'options', label: 'Divisions', kind: 'choice', default: 'both',
      choices: [{ value: 'both', label: 'Created and lost' }, { value: 'created', label: 'Created' }, { value: 'lost', label: 'Lost and transferred' }] },
    { key: 'templates', section: 'options', label: 'Template types', kind: 'content-set', default: '', automatic: 'All templates', visibleWhen: () => false },
    { key: 'totals', section: 'options', label: 'Totals', kind: 'toggle', default: false },
    columnsControl(DIVISION_LOG_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
  ],
} as const satisfies WidgetMeta;
