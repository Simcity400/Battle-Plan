import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'national-spirits', version: 1, title: 'National spirits', icon: 'star',
  description: 'The country’s national spirits and since when it has held each.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  minSize: { w: 3, h: 3 }, defaultSize: { w: 4, h: 5 },
} as const satisfies WidgetMeta;
