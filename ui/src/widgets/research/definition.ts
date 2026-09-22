import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'research', version: 1, title: 'Research', icon: 'grid',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'moment', data: true,
  minSize: { w: 3, h: 5 }, defaultSize: { w: 7, h: 10 },
} as const satisfies WidgetMeta;
