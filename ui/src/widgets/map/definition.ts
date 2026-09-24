import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'map', version: 1, title: 'Campaign map', icon: 'globe', description: 'The native map, following playback or pinned to a moment.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 3, h: 8 }, defaultSize: { w: 7, h: 11 },
} as const satisfies WidgetMeta;
