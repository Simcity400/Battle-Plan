import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION, choiceVariants } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION, breakdownVariants } from '../../charts/drill/drillState.ts';

export const TENSION_BREAKDOWN = { id: 'breakdown', label: 'Country contributions' };

const tension = (options: Readonly<Record<string, unknown>>) => (options.view ?? 'tension') === 'tension';
const stacked = (options: Readonly<Record<string, unknown>>) => tension(options) && (options.show ?? 'main') === 'main';
const overTime = (options: Readonly<Record<string, unknown>>) => tension(options) || options.view === 'lines';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'tension', choices: [
    { value: 'tension', label: 'World tension' }, { value: 'race', label: 'Contributions race' },
    { value: 'lines', label: 'Contributions over time' }, { value: 'events', label: 'Threat events' },
  ] },
  { ...breakdownOption([TENSION_BREAKDOWN], 'World tension'), visibleWhen: tension },
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in', visibleWhen: overTime,
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  { ...AFTER_PLAYHEAD_OPTION, visibleWhen: options => overTime(options) && options.playback === 'cursor' },
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => overTime(options) && options.playback === 'window' },
  { key: 'layers', section: 'options', label: 'Layers', hint: 'Ranked at the moment shown', kind: 'choice', default: '5',
    choices: [{ value: '5', label: 'Top 5 + Other' }, { value: '8', label: 'Top 8 + Other' }], visibleWhen: stacked },
  { key: 'values', section: 'options', label: 'Layers show', kind: 'choice', default: 'share', hint: 'Contributions can exceed 100%.',
    choices: [{ value: 'share', label: 'Share of world tension' }, { value: 'recorded', label: 'Contributions' }], visibleWhen: stacked },
  { key: 'markers', section: 'options', label: 'Event markers', kind: 'toggle', default: true, visibleWhen: stacked },
  { key: 'list', section: 'options', label: 'Values beside the chart', kind: 'toggle', default: true, visibleWhen: stacked },
  { ...DRILL_OPTION, visibleWhen: stacked },
];

export const definition = {
  kind: 'world-tension', version: 3, title: 'World tension & threat', icon: 'alert',
  description: 'World tension over time, stacked by the countries whose actions generated it, with the events that did.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true },
  options: OPTIONS,
  migrate: (_from, { options = {}, ...widget }) => ({ ...widget,
    options: { ...options, view: typeof options.view === 'string' && ['tension', 'race', 'lines', 'events'].includes(options.view) ? options.view : 'tension' } }),
  variants: [...breakdownVariants('World tension & threat', OPTIONS.filter(field => field.key === 'show')),
    ...choiceVariants(OPTIONS.find(field => field.key === 'values'), () => 'World tension contributions'),
    ...choiceVariants(OPTIONS.find(field => field.key === 'view'), choice => choice.value === 'tension' ? 'World tension & threat' : `World tension: ${choice.label}`)],
} as const satisfies WidgetMeta;
