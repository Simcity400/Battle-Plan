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

export const PROJECT_CHOICES = [
  { value: 'all', label: 'New and repairs' },
  { value: 'new', label: 'New buildings' },
  { value: 'repair', label: 'Repairs' },
] as const;

export const CONSTRUCTION_LOG_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ minWidth: 74 }),
  logKindColumn({ minWidth: 34, maxWidth: 42 }),
  logCountryColumn({ minWidth: 54 }),
  logItemColumn({ label: 'Building', minWidth: 72 }),
  logPlaceColumn({ minWidth: 112, priority: 3 }),
  {
    key: 'levels',
    label: 'Levels',
    as: 'plain',
    offered: OFFERED.text,
    priority: 2,
    sortable: false,
    numeric: true,
    minWidth: 120,
    widest: ['Levels not recorded'],
  },
  {
    key: 'factories',
    label: 'Factories',
    as: 'plain',
    offered: OFFERED.text,
    priority: 3,
    sortable: false,
    numeric: true,
    minWidth: 112,
    widest: ['999 factories'],
  },
  logStatusColumn({ minWidth: 116, widest: ['Repaired and built'] }),
];

export const OPTIONS: readonly OptionField[] = [
  {
    key: 'types',
    section: 'options',
    label: 'Building types',
    hint: 'Leave empty to show all building types.',
    kind: 'content-set',
    automatic: 'All buildings',
    default: '',
    visibleWhen: () => false,
  },
  { key: 'projects', section: 'options', label: 'Projects', kind: 'choice', default: 'all', choices: PROJECT_CHOICES },
  { key: 'queue', section: 'options', label: 'In the queue', hint: 'Only projects still in the queue at the moment shown.', kind: 'toggle', default: false },
  {
    key: 'order',
    section: 'options',
    label: 'Order',
    kind: 'choice',
    default: 'change',
    choices: [{ value: 'change', label: 'Latest change' }, { value: 'start', label: 'Start date' }],
  },
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  columnsControl(CONSTRUCTION_LOG_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
];

export const definition = {
  kind: 'construction-log',
  version: 2,
  title: 'Construction log',
  icon: 'hammer',
  description: 'Every construction project: what was built or repaired where, when it started and finished, and the factories on it.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 4, h: 6 },
  defaultSize: { w: 7, h: 14 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: { product: 'construction-log-tail', params: { limit: 1 } },
  options: OPTIONS,
} as const satisfies WidgetMeta;
