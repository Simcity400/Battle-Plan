import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'supply-health',
  version: 1,
  title: 'Supply health',
  icon: 'shield',
  description: 'The share of a country’s divisions in supply, strained or without supply over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 4, h: 6 },
  defaultSize: { w: 8, h: 10 },
  countries: { max: 1 },
} as const satisfies WidgetMeta;
