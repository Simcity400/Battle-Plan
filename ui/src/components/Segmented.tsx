import { Icon } from './Icons';
import type { IconName } from './Icons';

/** One choice from a short list, shown as a row of buttons. */
export function Segmented<T extends string>({ value, options, onChange, label }: {
  value: T; options: readonly { value: T; label: string; icon?: IconName; tooltip?: string }[]; onChange: (value: T) => void; label: string;
}) {
  return <div className="segmented" role="group" aria-label={label}>{options.map(option => <button key={option.value} type="button"
    aria-pressed={value === option.value} data-tooltip={option.tooltip} onClick={() => onChange(option.value)}>
    {option.icon && <Icon name={option.icon} />}{option.label}</button>)}</div>;
}
