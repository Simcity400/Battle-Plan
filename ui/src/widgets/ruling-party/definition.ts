import type { WidgetMeta } from '../../dashboard/definitions';
import { AFTER_PLAYHEAD_OPTION } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'playback', section: 'time', label: 'While following playback', kind: 'choice', default: 'draw-in',
    choices: [{ value: 'draw-in', label: 'Grow to now' }, { value: 'cursor', label: 'Whole campaign' }, { value: 'window', label: 'Recent year' }] },
  AFTER_PLAYHEAD_OPTION,
  { key: 'changes', section: 'options', label: 'Government changes and elections', kind: 'toggle', default: true },
  { key: 'leaders', section: 'options', label: 'Leader changes', kind: 'toggle', default: true },
];

export const definition = {
  kind: 'ruling-party', version: 1, title: 'Ruling party', icon: 'palette',
  description: 'The country leader and party popularity over time, with changes of government, leader and elections.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
