import type { FactoryCount } from '../generated/dashboard';

/** Factory counts use the same native formulas in country and faction views. */
export const COUNT_MODES: readonly { value: FactoryCount; label: string; hint: string }[] = [
  { value: 'free', label: 'Available', hint: 'Built less damaged, consumer goods, trade, national projects, licences, market contracts and factories given away' },
  { value: 'built', label: 'Built', hint: 'The game’s Total: every factory counted, damaged ones included' },
  { value: 'topBar', label: 'Top bar', hint: 'The top bar’s group total: built less damaged and factories given away' },
];

export function factoryCountOf(value: unknown): FactoryCount {
  return COUNT_MODES.find(mode => mode.value === value)?.value ?? 'free';
}

export function factoryMeasureLabel(mode: FactoryCount): string {
  return `${COUNT_MODES.find(item => item.value === mode)!.label} factories`;
}
