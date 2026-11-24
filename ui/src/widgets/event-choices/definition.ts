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
import type { ScriptEventKind } from '../../generated/dashboard';

export const EVENT_KINDS: readonly { value: string; label: string; event: ScriptEventKind }[] = [
  { value: 'countryEvents', label: 'Country events', event: 'country' },
  { value: 'newsEvents', label: 'News events', event: 'news' },
];
export const AWAITING = { value: 'awaiting', label: 'Awaiting choice only', hint: 'Only events still awaiting a choice at the moment shown.' } as const;

export const EVENT_CHOICES_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn(),
  logKindColumn({ minWidth: 44, maxWidth: 50 }),
  logCountryColumn({ minWidth: 52 }),
  logItemColumn({ label: 'Event', minWidth: 200 }),
  {
    key: 'choice',
    label: 'Choice',
    as: 'plain',
    offered: OFFERED.text,
    priority: 1,
    sortable: false,
    text: true,
    grow: 1,
    minWidth: 180,
  },
  logActionColumn(),
];

export const OPTIONS: readonly OptionField[] = [
  ...EVENT_KINDS.map(kind => ({ key: kind.value, section: 'options' as const, label: kind.label, kind: 'toggle' as const, default: true })),
  { key: AWAITING.value, section: 'options', label: AWAITING.label, hint: AWAITING.hint, kind: 'toggle', default: false },
  { key: 'artwork', section: 'options', label: 'Game artwork', kind: 'toggle', default: true },
  columnsControl(EVENT_CHOICES_COLUMNS, LOG_TABLE_BASE, { key: TABLE_SETTINGS_KEY }),
];

export const definition = {
  kind: 'event-choices',
  version: 2,
  title: 'Event choices',
  icon: 'ballot',
  category: 'Politics',
  description: 'Events each country received, the option it chose and how long it took.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 6, h: 8 },
  defaultSize: { w: 12, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
} as const satisfies WidgetMeta;
