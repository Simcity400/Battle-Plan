import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'active-research', version: 1, title: 'Active research', icon: 'flask',
  description: 'What each research slot is researching now, with its progress and the game’s days-left estimate, and idle slots with their saved days.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  minSize: { w: 3, h: 3 }, defaultSize: { w: 7, h: 5 },
} as const satisfies WidgetMeta;
