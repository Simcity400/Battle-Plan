import type { WidgetMeta } from '../../dashboard/definitions';
import { choiceVariants } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { DATE_OVERLAY_OPTION } from '../../charts/race/options.ts';
import { OFFERED, tableFamilyOptions, type TableBase, type TableColumnDefaults } from '../../families/table/settings.ts';

const tension = (options: Readonly<Record<string, unknown>>) => (options.view ?? 'tension') === 'tension';
const events = (options: Readonly<Record<string, unknown>>) => options.view === 'events';
const race = (options: Readonly<Record<string, unknown>>) => options.view === 'race';

export const EVENTS_TABLE_BASE: TableBase = {
  density: 'compact',
  barsBehind: 'off',
  overflow: 'scroll',
  showRank: false,
  sort: { column: 'date', direction: 'descending' },
};

const EVENTS_COLUMN_DEFAULTS: readonly TableColumnDefaults[] = [
  { key: 'date', label: 'Created', as: 'plain', offered: OFFERED.text },
  { key: 'country', label: 'Country', as: 'flagName', offered: OFFERED.country, showNamesDefault: true },
  { key: 'reason', label: 'Event', as: 'plain', offered: OFFERED.text, hideable: false },
  { key: 'added', label: 'Adds', as: 'faint', offered: OFFERED.number },
  { key: 'until', label: 'Listed until', as: 'plain', offered: OFFERED.text },
];

export const EVENTS_TABLE_OPTIONS: readonly OptionField[] = tableFamilyOptions(EVENTS_COLUMN_DEFAULTS, EVENTS_TABLE_BASE).map(field => ({
  ...field,
  visibleWhen: events,
}));

export const TENSION_CHART_OPTIONS: readonly OptionField[] = [
  { key: 'layers', section: 'options', label: 'Layers', hint: 'Ranked at the moment shown', kind: 'choice', default: '5',
    choices: [{ value: '5', label: 'Top 5 + Other' }, { value: '8', label: 'Top 8 + Other' }], visibleWhen: tension },
  { key: 'values', section: 'options', label: 'Layers show', kind: 'choice', default: 'share', hint: 'Contributions can exceed 100%.',
    choices: [{ value: 'share', label: 'Share of world tension' }, { value: 'recorded', label: 'Contributions' }], visibleWhen: tension },
  { key: 'markers', section: 'options', label: 'Event markers', kind: 'toggle', default: true, visibleWhen: tension },
];

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'tension', choices: [
    { value: 'tension', label: 'World tension' }, { value: 'race', label: 'Contributions race' },
    { value: 'lines', label: 'Contributions over time' }, { value: 'events', label: 'Threat events' },
  ] },
  ...TENSION_CHART_OPTIONS,
  ...EVENTS_TABLE_OPTIONS,
  { ...DATE_OVERLAY_OPTION, visibleWhen: race },
];

export const definition = {
  kind: 'world-tension', version: 4, title: 'World tension', icon: 'chart-line',
  description: 'World tension over time, stacked by the countries whose actions generated it, with the events that did.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true },
  options: OPTIONS,
  viewSwitch: { option: 'view' },
  migrate: (_from, { options = {}, ...widget }) => ({
    ...widget,
    options: { ...options, view: typeof options.view === 'string' && ['tension', 'race', 'lines', 'events'].includes(options.view) ? options.view : 'tension' },
  }),
  variants: [
    ...choiceVariants(OPTIONS.find(field => field.key === 'values'), () => 'World tension contributions'),
    ...choiceVariants(OPTIONS.find(field => field.key === 'view'), choice => choice.value === 'tension' ? 'World tension' : `World tension: ${choice.label}`),
  ],
} as const satisfies WidgetMeta;
