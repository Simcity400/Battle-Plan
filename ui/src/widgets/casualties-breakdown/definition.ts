import type { WidgetMeta } from '../../dashboard/definitions';
import { NARROW_TABLE_OPTION } from '../../charts/table/options.ts';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, breakdownVariants, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';

export const DETAIL = { id: 'details', label: 'Recorded losses' };
export const OPTIONS: readonly OptionField[] = [levelOption('warLevel', 'War'), levelOption('enemyLevel', 'Enemy country'),
  ...SUNBURST_OPTIONS, DRILL_OPTION, breakdownOption([DETAIL], 'Sunburst'), NARROW_TABLE_OPTION];
export const definition = {
  kind: 'casualties-breakdown', version: 1, title: 'Casualties breakdown', icon: 'sunburst',
  description: 'A country’s losses by war and enemy.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS, variants: breakdownVariants('Casualties breakdown', OPTIONS),
} as const satisfies WidgetMeta;
