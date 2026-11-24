import type { WidgetMeta } from '../../dashboard/definitions';
import { NARROW_TABLE_OPTION } from '../../charts/table/options.ts';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION, breakdownVariants } from '../../charts/drill/drillState.ts';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';

export const DETAIL = { id: 'loss-details', label: 'Recorded losses' };

export const OPTIONS: readonly OptionField[] = [
  { key: 'landSource', section: 'options', label: 'Land losses from', kind: 'choice', default: 'destruction',
    choices: [{ value: 'destruction', label: 'Every destruction' }, { value: 'combatLog', label: "Game's Combat Log" }] },
  levelOption('causeLevel', 'Cause'),
  levelOption('typeLevel', 'Equipment type'),
  levelOption('designLevel', 'Exact design'),
  ...SUNBURST_OPTIONS, DRILL_OPTION, breakdownOption([DETAIL], 'Sunburst'), NARROW_TABLE_OPTION];

export const definition = {
  kind: 'equipment-losses', version: 1, title: 'Equipment losses', icon: 'sunburst',
  description: 'Equipment losses by cause, type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
  variants: breakdownVariants('Equipment losses', OPTIONS),
} as const satisfies WidgetMeta;
