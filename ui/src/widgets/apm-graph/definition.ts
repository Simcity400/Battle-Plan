import type { WidgetMeta } from '../../dashboard/definitions';
import { GRAPH_OPTIONS } from '../player-apm/definition';

export const definition = {
  kind: 'apm-graph', version: 1, title: 'eAPM graph', icon: 'chart-bars', category: 'Overview',
  description: 'Effective actions per minute (eAPM), by country or player. Counts accepted player-issued orders per unpaused real minute. Paused orders can be shown separately.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 9 }, options: GRAPH_OPTIONS,
} as const satisfies WidgetMeta;
