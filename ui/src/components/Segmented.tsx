import { Icon } from './Icons';
import type { IconName } from './Icons';

export function Segmented<T extends string>({ value, options, onChange, label, disabled = false }: {
  value: T; options: readonly { value: T; label: string; icon?: IconName; tooltip?: string; disabled?: boolean; reason?: string }[]; onChange: (value: T) => void; label: string; disabled?: boolean;
}) {
  return <div className="segmented" role="group" aria-label={label}>{options.map(option => {
    const blocked = disabled || !!option.disabled;
    const htmlDisabled = disabled || (!!option.disabled && !option.reason);
    return <button key={option.value} type="button" disabled={htmlDisabled} aria-disabled={blocked || undefined}
      aria-pressed={value === option.value} data-tooltip={option.disabled ? (option.reason ?? option.tooltip) : option.tooltip}
      onClick={() => { if (!blocked) onChange(option.value); }}>
      {option.icon && <Icon name={option.icon} />}{option.label}</button>;
  })}</div>;
}
