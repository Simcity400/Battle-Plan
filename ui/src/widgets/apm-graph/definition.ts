import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const APM_GRAPH_OPTIONS: readonly OptionField[] = [
  { key: 'group', section: 'options', label: 'Compare', kind: 'choice', default: 'player',
    choices: [{ value: 'player', label: 'Players' }, { value: 'country', label: 'Countries' }] },
  { key: 'players', section: 'options', label: 'Players', kind: 'content-set', catalogue: true,
    automatic: 'All players in the selected countries', default: '', visibleWhen: options => options.group !== 'country' },
  { key: 'yMin', section: 'axes', label: 'Value axis from', hint: 'Empty starts at zero', kind: 'number', min: 0, max: 1e9, step: 1, default: null },
  { key: 'yMax', section: 'axes', label: 'Value axis to', hint: 'Empty fits the data', kind: 'number', min: 0, max: 1e9, step: 1, default: null },
];

export const definition = {
  kind: 'apm-graph', version: 2, title: 'eAPM graph', icon: 'chart-line', category: 'Overview',
  description: 'Effective actions per minute (eAPM), by player or country. Counts accepted player-issued orders per unpaused real minute.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 9 }, options: APM_GRAPH_OPTIONS,
} as const satisfies WidgetMeta;
