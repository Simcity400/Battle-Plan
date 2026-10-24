import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'national-focus', version: 1, title: 'National focus', icon: 'target', description: 'Completed focuses in order and the one in progress.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  minSize: { w: 3, h: 5 }, defaultSize: { w: 5, h: 10 },
} as const satisfies WidgetMeta;
