import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'map', version: 1, title: 'Campaign map', icon: 'globe',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true,
  minSize: { w: 3, h: 8 }, defaultSize: { w: 7, h: 11 },
} as const satisfies WidgetMeta;
