import { useId } from 'react';
import { Icon } from '../components/Icons';
import { Popover, usePopover } from '../components/Popover';
import { Segmented } from '../components/Segmented';

export interface WidgetViewSwitchProps {
  label: string;
  value: string;
  options: readonly { value: string; label: string }[];
  compact?: boolean;
  readOnly?: boolean;
  onChange: (value: string) => void;
}

export default function WidgetViewSwitch({ label, value, options, compact = false, readOnly = false, onChange }: WidgetViewSwitchProps) {
  const id = useId();
  const menu = usePopover('end');
  if (options.length <= 1) return null;
  const asMenu = compact || options.length >= 4;
  if (!asMenu) {
    return <div className="dash-view-switch" data-view-switch="">
      <Segmented label={label} value={value} options={options} disabled={readOnly} onChange={readOnly ? () => {} : onChange} />
    </div>;
  }
  const current = options.find(option => option.value === value) ?? options[0];
  if (readOnly) {
    return <span className="dash-view-menu-trigger is-static" data-view-switch="">
      <span>{current.label}</span>
    </span>;
  }
  return <>
    <button ref={menu.trigger} type="button" className="dash-view-menu-trigger" data-view-switch="" popoverTarget={id}
      aria-haspopup="menu" aria-expanded={menu.open} aria-label={`${label}: ${current.label}`} data-tooltip={label}>
      <span>{current.label}</span>
      <Icon name="chevron-down" className="dash-view-menu-chevron" />
    </button>
    <Popover menu={menu} id={id} role="menu" className="dash-view-menu" aria-label={label}>
      {menu.open && options.map(option => <button key={option.value} type="button" role="menuitemradio" className="menu-item"
        aria-checked={option.value === value}
        onClick={() => { menu.close(); if (option.value !== value) onChange(option.value); }}>
        <span className="menu-item-label">{option.label}</span>
        {option.value === value && <Icon name="check" />}
      </button>)}
    </Popover>
  </>;
}
