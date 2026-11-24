import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'balance-of-power',
  version: 1,
  title: 'Balance of power',
  category: 'Politics',
  icon: 'chart-line',
  description: 'A country’s internal balance of power over time: where it stands, its ranges and what moves it.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 4, h: 6 },
  defaultSize: { w: 12, h: 10 },
  countries: { max: 1 },
  defaultCountries: { kind: 'selected' },
} as const satisfies WidgetMeta;
