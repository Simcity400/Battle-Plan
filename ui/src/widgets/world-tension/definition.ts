import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';

/** Settings as signed off (mock-up diplo-1): Top 5 + Other layers by default or Top
 * 8, event markers on, and the values listed beside the chart. What the layers
 * show is a setting too: each country's share of world tension (the look signed
 * off: the layers' top edge is world tension) or its recorded contribution, which
 * can add up past the game's 100% cap. */
export const OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent window' }] },
  AFTER_PLAYHEAD_OPTION,
  { key: 'windowDays', section: 'time', label: 'Window length', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365,
    visibleWhen: options => options.playback === 'window' },
  { key: 'layers', section: 'options', label: 'Layers', hint: 'Ranked at the moment shown', kind: 'choice', default: '5',
    choices: [{ value: '5', label: 'Top 5 + Other' }, { value: '8', label: 'Top 8 + Other' }] },
  { key: 'values', section: 'options', label: 'Layers show', kind: 'choice', default: 'share',
    hint: 'A share adds up to world tension; recorded contributions can add up past its 100% cap',
    choices: [{ value: 'share', label: 'Share of world tension' }, { value: 'recorded', label: 'Recorded contributions' }] },
  { key: 'markers', section: 'options', label: 'Event markers', hint: 'Triangles along the top, in the causing country’s colour', kind: 'toggle', default: true },
  { key: 'list', section: 'options', label: 'Values beside the chart', hint: 'World tension and each layer at the moment shown', kind: 'toggle', default: true },
];

export const definition = {
  kind: 'world-tension', version: 1, title: 'World tension & threat', icon: 'alert',
  description: 'World tension over time, stacked by the countries whose actions generated it, with the events that did.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 10 },
  // Every country by default; chosen countries or a group narrow the layers to them.
  countries: { max: 64, all: true },
  options: OPTIONS,
} as const satisfies WidgetMeta;
