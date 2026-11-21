import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'active-research', version: 1, title: 'Active research', icon: 'flask',
  description: 'What each research slot is researching now: its progress, the game’s days-left estimate, research ahead of its year and idle slots.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  minSize: { w: 3, h: 3 }, defaultSize: { w: 7, h: 9 }, playsBackDated: true,
} as const satisfies WidgetMeta;
