import type { WidgetMeta } from '../../dashboard/definitions';
import { TABLE_APM_OPTIONS } from '../player-apm/definition.ts';

export const definition = {
  kind: 'apm-table', version: 1, title: 'eAPM table', icon: 'chart-bars', category: 'Overview',
  description: 'Compare eAPM and orders over a selected period. eAPM always excludes paused orders and paused time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 10 }, options: TABLE_APM_OPTIONS,
} as const satisfies WidgetMeta;
