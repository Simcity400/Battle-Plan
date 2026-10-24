import type { WidgetMeta } from '../../dashboard/definitions';
import { battleOptions, battleVariants } from '../battles/battleOptions.ts';

export const OPTIONS = battleOptions([
  { value: 'Ships sunk', label: 'Ships sunk' }, { value: 'Peak ships', label: 'Peak ships' }, { value: 'Length', label: 'Length' },
]);
export const definition = {
  kind: 'naval-battles', version: 1, title: 'Naval battles', icon: 'shield',
  description: 'Sea battles, ships sunk and results.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: { product: 'naval-battles-manifest', params: {} }, options: OPTIONS,
  variants: battleVariants('Naval battles', OPTIONS),
} as const satisfies WidgetMeta;
