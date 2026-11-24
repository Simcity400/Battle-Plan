import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { columnsControl } from '../../families/settingsControls.ts';
import {
  LOG_TABLE_BASE,
  logActionColumn,
  logCountryColumn,
  logDateColumn,
  logItemColumn,
  logKindColumn,
} from '../../families/table/logHelpers.ts';
import type { TableColumnSpec } from '../../families/table/model.ts';
import { OFFERED, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';

export const CATEGORIES = [
  { value: 'warsPeace', label: 'Wars & peace', palette: 1 },
  { value: 'diplomacy', label: 'Diplomacy', palette: 0 },
  { value: 'politics', label: 'Politics', palette: 5 },
  { value: 'milestones', label: 'Milestones', palette: 2 },
] as const;

export const EVENT_FEED_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn(),
  logKindColumn({ minWidth: 44, maxWidth: 50 }),
  logCountryColumn({ minWidth: 54 }),
  logItemColumn({ label: 'Event', minWidth: 220 }),
  {
    key: 'detail',
    label: 'Detail',
    as: 'plain',
    offered: OFFERED.text,
    priority: 2,
    sortable: false,
    text: true,
    grow: 1,
    minWidth: 150,
  },
  logActionColumn(),
];

export const OPTIONS: readonly OptionField[] = [
  ...CATEGORIES.map(category => ({
    key: category.value,
    section: 'options' as const,
    label: category.label,
    kind: 'toggle' as const,
    default: true,
  })),
  { key: 'activityStrip', section: 'options', label: 'Activity strip', kind: 'toggle', default: true },
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  columnsControl(EVENT_FEED_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
];

export const definition = {
  kind: 'event-feed',
  version: 2,
  title: 'Event feed',
  icon: 'clock',
  description: 'Key moments across every country.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 4, h: 6 },
  defaultSize: { w: 7, h: 14 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
} as const satisfies WidgetMeta;
