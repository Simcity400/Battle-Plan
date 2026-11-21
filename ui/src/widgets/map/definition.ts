import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'map', version: 1, title: 'Campaign map', icon: 'globe', description: 'Campaign map at the selected time.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  captureModes: ['full', 'map'],
  minSize: { w: 3, h: 8 }, defaultSize: { w: 7, h: 11 },
  preview: false,
} as const satisfies WidgetMeta;
