import type { OptionField, OptionValue } from '../../dashboard/options';

export const BREAKDOWN_VIEWS = [
  { value: 'parts', label: 'Parts' },
  { value: 'ring', label: 'Ring' },
  { value: 'mosaic', label: 'Mosaic' },
] as const;

export const BREAKDOWN_MONTH_VIEWS = [
  ...BREAKDOWN_VIEWS,
  { value: 'months', label: 'By month' },
] as const;

export type BreakdownViewMode = (typeof BREAKDOWN_VIEWS)[number]['value'];
export type BreakdownOrMonthView = (typeof BREAKDOWN_MONTH_VIEWS)[number]['value'];

export const isBreakdownRingView = (options: Readonly<Record<string, OptionValue>>): boolean =>
  breakdownViewOf(options) === 'ring';

export const isNotMonthView = (options: Readonly<Record<string, OptionValue>>): boolean =>
  options.view !== 'months';

export function breakdownViewOf(options: Readonly<Record<string, OptionValue>>): BreakdownViewMode {
  if (options.view === 'ring' || options.view === 'circle') return 'ring';
  if (options.view === 'mosaic') return 'mosaic';
  return 'parts';
}

export const BREAKDOWN_VIEW_OPTION: OptionField = {
  key: 'view',
  section: 'options',
  label: 'View',
  kind: 'choice',
  default: 'parts',
  choices: BREAKDOWN_VIEWS,
};

export const BREAKDOWN_MONTH_VIEW_OPTION: OptionField = {
  key: 'view',
  section: 'options',
  label: 'View',
  kind: 'choice',
  default: 'parts',
  choices: BREAKDOWN_MONTH_VIEWS,
};

export const NAMES_IN_RING_OPTION: OptionField = {
  key: 'namesInRing',
  section: 'options',
  label: 'Names in ring',
  kind: 'toggle',
  default: true,
  visibleWhen: isBreakdownRingView,
};

export const BREAKDOWN_OPTIONS: readonly OptionField[] = [
  BREAKDOWN_VIEW_OPTION,
  NAMES_IN_RING_OPTION,
];

export const BREAKDOWN_MONTH_OPTIONS: readonly OptionField[] = [
  BREAKDOWN_MONTH_VIEW_OPTION,
  NAMES_IN_RING_OPTION,
];

export const levelOption = (key: string, label: string, hint?: string): OptionField =>
  ({ key, section: 'options', label, hint, kind: 'toggle', default: true });

export const levelsShown = (options: Readonly<Record<string, OptionValue>>, keys: readonly string[]): boolean[] =>
  keys.map(key => options[key] !== false);
