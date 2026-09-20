import { Popover, usePopover } from '../ui/Popover';
import { useId } from 'react';
import { Icon } from '../Icons';
import { widgetDefinitions } from './definitions';

export default function WidgetPicker({ disabled, onAdd, onOpen }: { disabled: boolean; onAdd: (kind: string) => void; onOpen: () => void }) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;

  return <>
    <button ref={trigger} type="button" className="button quiet header-add-widget" aria-label="Add widget" data-tooltip="Add widget" disabled={disabled}
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open} onClick={onOpen}><Icon name="plus" /></button>
    <Popover menu={menu} id={id} className="widget-picker" aria-label="Add a widget">
      <div className="widget-picker-heading"><strong>Add a widget</strong><button type="button" className="dash-icon-button" aria-label="Close widget picker" popoverTarget={id} popoverTargetAction="hide"><Icon name="close" /></button></div>
      <div className="widget-picker-items">{widgetDefinitions.map((definition) => <button type="button" key={definition.kind} disabled={disabled} onClick={() => {
        menu.close(); onAdd(definition.kind);
      }}><Icon name={definition.kind === 'map' ? 'globe' : 'grid'} /><span>{definition.kind === 'placeholder' ? 'Empty panel' : definition.title}</span><Icon name="plus" /></button>)}</div>
    </Popover>
  </>;
}
