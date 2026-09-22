import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'national-focus', version: 1, title: 'National focus', icon: 'grid',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'moment', data: true,
  minSize: { w: 3, h: 5 }, defaultSize: { w: 5, h: 10 },
} as const satisfies WidgetMeta;
