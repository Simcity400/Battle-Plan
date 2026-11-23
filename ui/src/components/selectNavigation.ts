import type { SelectOption } from './Select';
export const selectFocusable = (option: Pick<SelectOption, 'disabled' | 'reason'>): boolean => !option.disabled || !!option.reason;
export function selectIndex(options: readonly Pick<SelectOption, 'disabled' | 'reason'>[], at: number, key: string): number | null {
  if (!options.length) return null;
  const step = key === 'ArrowDown' || key === 'Home' ? 1 : key === 'ArrowUp' || key === 'End' ? -1 : 0;
  if (!step) return null;
  let next = key === 'Home' ? -1 : key === 'End' ? options.length : at;
  for (let i = 0; i < options.length; i++) { next = (next + step + options.length) % options.length; if (selectFocusable(options[next])) return next; }
  return null;
}
