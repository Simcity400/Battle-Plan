import { Popover, usePopover } from '../components/Popover';
import { useId } from 'react';
import { Icon } from '../components/Icons';
import { widgetDefinitions } from './definitions';

export default function WidgetPicker({ disabled, onAdd, onOpen }: { disabled: boolean; onAdd: (kind: string) => void; onOpen: () => void }) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;

  return <>
    <button ref={trigger} type="button" className="icon-button header-add-widget" aria-label="Add widget" data-tooltip="Add a widget" disabled={disabled}
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open} onClick={onOpen}><Icon name="plus" /></button>
    <Popover menu={menu} id={id} className="widget-picker" aria-label="Add a widget">
      <div className="ui-popover-heading"><strong>Add a widget</strong><button type="button" className="icon-button ui-popover-action" aria-label="Close widget picker" popoverTarget={id} popoverTargetAction="hide"><Icon name="close" /></button></div>
      <div className="widget-picker-items">{widgetDefinitions().map((definition) => <button type="button" key={definition.kind} disabled={disabled} onClick={() => {
        menu.close(); onAdd(definition.kind);
      }}><span className="dash-widget-icon" aria-hidden="true"><Icon name={definition.icon} /></span>
        <span><strong>{definition.title}</strong>{definition.description && <small>{definition.description}</small>}</span><Icon name="plus" /></button>)}</div>
    </Popover>
  </>;
}
