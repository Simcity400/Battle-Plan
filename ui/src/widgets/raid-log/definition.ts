import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { columnsControl } from '../../families/settingsControls.ts';
import {
  LOG_TABLE_BASE,
  logCountryColumn,
  logDateColumn,
  logItemColumn,
  logKindColumn,
  logPlaceColumn,
  logStatusColumn,
} from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { OFFERED, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const RAID_LOG_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn(),
  logKindColumn({ minWidth: 34, maxWidth: 42 }),
  logCountryColumn({ minWidth: 54 }),
  logItemColumn({ label: 'Raid', minWidth: 180 }),
  logPlaceColumn({ minWidth: 110 }),
  {
    key: 'preparation',
    label: 'Preparation',
    as: 'plain',
    offered: OFFERED.text,
    priority: 2,
    sortable: false,
    numeric: true,
    minWidth: 136,
    widest: ['999 days preparing'],
  },
  logStatusColumn({ minWidth: 128, widest: ['Ready to launch'] }),
];

export const OPTIONS: readonly OptionField[] = [
  {
    key: 'types',
    section: 'options',
    label: 'Raid types',
    hint: 'Leave empty to show all types.',
    kind: 'content-set',
    automatic: 'All raid types',
    default: '',
    visibleWhen: () => false,
  },
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  columnsControl(RAID_LOG_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
];

export const definition = {
  kind: 'raid-log',
  version: 2,
  title: 'Raid log',
  icon: 'target',
  description: 'Raids, targets, preparation, phases and outcomes.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 4, h: 6 },
  defaultSize: { w: 7, h: 14 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: { product: 'raid-log-tail', params: { limit: 1 } },
  options: OPTIONS,
} as const satisfies WidgetMeta;
