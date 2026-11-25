import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { columnsControl } from '../../families/settingsControls.ts';
import {
  LOG_TABLE_BASE,
  logCountryColumn,
  logDateColumn,
  logItemColumn,
  logKindColumn,
  logStatusColumn,
} from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { OFFERED, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const ESPIONAGE_LOG_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ minWidth: 74 }),
  logKindColumn({ minWidth: 42, maxWidth: 48 }),
  logCountryColumn({ minWidth: 64 }),
  logItemColumn({ label: 'Operation', minWidth: 160 }),
  {
    key: 'progress',
    label: 'Progress',
    as: 'plain',
    cellKind: 'track',
    trackTone: 'accent',
    offered: OFFERED.text,
    priority: 3,
    sortable: false,
    numeric: true,
    minWidth: 176,
    widest: ['157 days left'],
  },
  logStatusColumn({ priority: 1, minWidth: 108, widest: ['Completed'] }),
];

export const OPTIONS: readonly OptionField[] = [
  { key: 'types', section: 'options', label: 'Operation types', kind: 'content-set', automatic: 'Every type', default: '', visibleWhen: () => false },
  {
    key: 'countryRole',
    section: 'options',
    label: 'Match chosen countries as',
    kind: 'choice',
    default: 'owner',
    choices: [
      { value: 'owner', label: 'Owner' },
      { value: 'target', label: 'Target' },
      { value: 'either', label: 'Owner or target' },
    ],
  },
  {
    key: 'runningOrder',
    section: 'options',
    label: 'Running operations',
    hint: 'Active operations appear first while Live.',
    kind: 'choice',
    default: 'start',
    choices: [
      { value: 'start', label: 'Newest start first' },
      { value: 'preparation', label: 'Highest preparation first' },
    ],
  },
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  columnsControl(ESPIONAGE_LOG_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
];

export const definition = {
  kind: 'espionage-log',
  version: 2,
  title: 'Espionage operations',
  icon: 'eye',
  description: 'Espionage operations, preparation and outcomes.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  minSize: { w: 4, h: 6 },
  defaultSize: { w: 9, h: 14 },
  availability: { product: 'espionage-log-tail', params: { limit: 1 } },
  options: OPTIONS,
} as const satisfies WidgetMeta;
