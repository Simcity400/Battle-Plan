import type { WidgetMeta } from '../../dashboard/definitions';
import { NARROW_TABLE_OPTION } from '../../charts/table/options.ts';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION, breakdownVariants } from '../../charts/drill/drillState.ts';
import { BREAKDOWN_OPTIONS, levelOption } from '../../families/breakdown/options.ts';

export const DETAIL = { id: 'loss-details', label: 'Recorded losses' };

export const OPTIONS: readonly OptionField[] = [
  ...BREAKDOWN_OPTIONS,
  {
    key: 'landSource',
    section: 'options',
    label: 'Land losses from',
    kind: 'choice',
    default: 'destruction',
    choices: [{ value: 'destruction', label: 'Every destruction' }, { value: 'combatLog', label: 'Combat Log' }],
  },
  levelOption('causeLevel', 'Cause'),
  levelOption('typeLevel', 'Equipment type'),
  levelOption('designLevel', 'Exact design'),
  DRILL_OPTION,
  breakdownOption([DETAIL], 'Breakdown'),
  NARROW_TABLE_OPTION,
];

export const definition = {
  kind: 'equipment-losses', version: 1, title: 'Equipment losses', icon: 'sunburst',
  description: 'Equipment losses by cause, type and design.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  playsBackDated: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS, viewSwitch: { option: 'view' },
  variants: breakdownVariants('Equipment losses', OPTIONS),
} as const satisfies WidgetMeta;
