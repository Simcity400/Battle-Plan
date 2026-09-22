import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'placeholder', version: 1, title: 'Untitled panel', icon: 'grid',
  timeModes: ['campaign', 'follow'], defaultTime: 'campaign', scope: 'moment', data: false,
  minSize: { w: 2, h: 3 }, defaultSize: { w: 4, h: 6 },
} as const satisfies WidgetMeta;
