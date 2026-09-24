import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'research', version: 1, title: 'Research', icon: 'flask', description: 'Each research slot with its assignment and completed technologies.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'campaign', scope: 'moment', data: true, history: true,
  minSize: { w: 3, h: 5 }, defaultSize: { w: 7, h: 10 },
} as const satisfies WidgetMeta;
