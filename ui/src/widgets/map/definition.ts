import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'map', version: 1, title: 'Campaign map', icon: 'globe', description: 'Campaign map at the selected time.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 3, h: 8 }, defaultSize: { w: 7, h: 11 },
  // The native map draws one view per widget on the page; the catalogue describes it instead.
  preview: false,
} as const satisfies WidgetMeta;
