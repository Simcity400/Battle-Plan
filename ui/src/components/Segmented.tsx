import { Icon } from './Icons';
import type { IconName } from './Icons';

export function Segmented<T extends string>({ value, options, onChange, label, disabled = false }: {
  value: T; options: readonly { value: T; label: string; icon?: IconName; tooltip?: string }[]; onChange: (value: T) => void; label: string; disabled?: boolean;
}) {
  return <div className="segmented" role="group" aria-label={label}>{options.map(option => <button key={option.value} type="button" disabled={disabled}
    aria-pressed={value === option.value} data-tooltip={option.tooltip} onClick={() => onChange(option.value)}>
    {option.icon && <Icon name={option.icon} />}{option.label}</button>)}</div>;
}
