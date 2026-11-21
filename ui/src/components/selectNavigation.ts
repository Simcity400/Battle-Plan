import type { SelectOption } from './Select';
export function selectIndex(options: readonly Pick<SelectOption, 'disabled'>[], at: number, key: string): number | null {
  if (!options.length) return null;
  const step = key === 'ArrowDown' || key === 'Home' ? 1 : key === 'ArrowUp' || key === 'End' ? -1 : 0;
  if (!step) return null;
  let next = key === 'Home' ? -1 : key === 'End' ? options.length : at;
  for (let i = 0; i < options.length; i++) { next = (next + step + options.length) % options.length; if (!options[next].disabled) return next; }
  return null;
}
