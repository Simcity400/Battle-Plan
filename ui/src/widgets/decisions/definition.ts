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
import { STATUS_CHOICES } from './decisionsData.ts';

export const DECISIONS_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn({ sortable: true }),
  logKindColumn({ minWidth: 34, maxWidth: 42 }),
  logCountryColumn({ sortable: true, minWidth: 54 }),
  logItemColumn({ sortable: true, label: 'Decision', minWidth: 180 }),
  logPlaceColumn({ minWidth: 92 }),
  {
    key: 'timer',
    label: 'Time left',
    as: 'plain',
    cellKind: 'track',
    trackTone: 'accent',
    offered: OFFERED.text,
    priority: 2,
    sortable: false,
    minWidth: 152,
    widest: ['999 days left'],
  },
  {
    key: 'cost',
    label: 'Cost',
    as: 'plain',
    offered: OFFERED.text,
    priority: 2,
    sortable: true,
    numeric: true,
    minWidth: 96,
    widest: ['999 PP'],
  },
  logStatusColumn({ sortable: true, minWidth: 120, widest: ['Completed'] }),
];

export const OPTIONS: readonly OptionField[] = [
  ...STATUS_CHOICES.map(status => ({
    key: status.value,
    section: 'options' as const,
    label: status.label,
    ...('hint' in status ? { hint: status.hint } : {}),
    kind: 'toggle' as const,
    default: true,
  })),
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  columnsControl(DECISIONS_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
];

export const definition = {
  kind: 'decisions',
  version: 2,
  title: 'Decisions',
  icon: 'ballot',
  category: 'Politics',
  description: 'Decisions and missions each country took, what they cost and how they ended.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 4, h: 6 },
  defaultSize: { w: 7, h: 14 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'selected' },
  options: OPTIONS,
} as const satisfies WidgetMeta;
