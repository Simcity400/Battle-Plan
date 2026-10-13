import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION, choiceVariants } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION, breakdownVariants } from '../../charts/drill/drillState.ts';
import { TENSION_BREAKDOWN } from './tensionDrill.ts';

/** Settings: Top 5 + Other layers by default or Top
 * 8, event markers on, and the values listed beside the chart. What the layers
 * show is a setting too: each country's share of world tension (the layers'
 * top edge is world tension) or its recorded contribution, which
 * can add up past the game's 100% cap. */
export const OPTIONS: readonly OptionField[] = [
  breakdownOption([TENSION_BREAKDOWN], 'World tension'),
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  AFTER_PLAYHEAD_OPTION,
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => options.playback === 'window' },
  { key: 'layers', section: 'options', label: 'Layers', hint: 'Ranked at the moment shown', kind: 'choice', default: '5',
    choices: [{ value: '5', label: 'Top 5 + Other' }, { value: '8', label: 'Top 8 + Other' }], visibleWhen: options => options.show === 'main' },
  { key: 'values', section: 'options', label: 'Layers show', kind: 'choice', default: 'share',
    hint: 'Contributions can exceed 100%.',
    choices: [{ value: 'share', label: 'Share of world tension' }, { value: 'recorded', label: 'Contributions' }], visibleWhen: options => options.show === 'main' },
  { key: 'markers', section: 'options', label: 'Event markers', kind: 'toggle', default: true,
    visibleWhen: options => options.show === 'main' },
  { key: 'list', section: 'options', label: 'Values beside the chart', kind: 'toggle', default: true,
    visibleWhen: options => options.show === 'main' },
  { ...DRILL_OPTION, visibleWhen: options => options.show === 'main' },
];

export const definition = {
  kind: 'world-tension', version: 1, title: 'World tension & threat', icon: 'alert',
  description: 'World tension over time, stacked by the countries whose actions generated it, with the events that did.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  // Every country by default; chosen countries or a group narrow the layers to them.
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
  variants: [...breakdownVariants('World tension & threat', OPTIONS),
    ...choiceVariants(OPTIONS.find(field => field.key === 'values'), () => 'World tension contributions')],
} as const satisfies WidgetMeta;
