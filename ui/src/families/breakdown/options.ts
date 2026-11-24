import type { OptionField, OptionValue } from '../../dashboard/options';

export const BREAKDOWN_VIEWS = [
  { value: 'parts', label: 'Parts' },
  { value: 'ring', label: 'Ring' },
  { value: 'mosaic', label: 'Mosaic' },
] as const;

export type BreakdownViewMode = (typeof BREAKDOWN_VIEWS)[number]['value'];

export function breakdownView(options: Readonly<Record<string, OptionValue>>): BreakdownViewMode {
  return BREAKDOWN_VIEWS.find(view => view.value === options.view)?.value ?? 'parts';
}

const ringShown = (options: Readonly<Record<string, OptionValue>>): boolean => breakdownView(options) === 'ring';

export const BREAKDOWN_OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'parts', choices: BREAKDOWN_VIEWS },
  { key: 'namesInRing', section: 'options', label: 'Names in ring', kind: 'toggle', default: true, visibleWhen: ringShown },
];
