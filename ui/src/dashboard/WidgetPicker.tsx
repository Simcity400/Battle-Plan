import { Popover, usePopover } from '../components/Popover';
import { useId } from 'react';
import { Icon } from '../components/Icons';
import { widgetDefinitions } from './definitions';
import type { WidgetPickerItemsProps, WidgetRuntime } from './definitions';
import { useProduct } from './products';
import { availabilityReason, availabilityRequest, validateWidgetAvailability } from './widgetAvailability';
import WidgetPickerItem from './WidgetPickerItem';

function PickerItem({ definition, runtime, track, active, disabled, onAdd }: WidgetPickerItemsProps) {
  const { data, error } = useProduct(runtime.client, availabilityRequest(definition, track), active, validateWidgetAvailability);
  const reason = availabilityReason(!!definition.availability, !!track, data, error);
  return <WidgetPickerItem definition={definition} disabled={disabled} reason={reason} onAdd={onAdd} />;
}

export default function WidgetPicker({ disabled, onAdd, onOpen, runtime }: { disabled: boolean; onAdd: WidgetPickerItemsProps['onAdd']; onOpen: () => void; runtime: WidgetRuntime }) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const track = runtime.tracks.find(item => item.roleId === runtime.defaultRoleId);

  return <>
    <button ref={trigger} type="button" className="icon-button header-add-widget" aria-label="Add widget" data-tooltip="Add a widget" disabled={disabled}
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open} onClick={onOpen}><Icon name="plus" /></button>
    <Popover menu={menu} id={id} className="widget-picker" aria-label="Add a widget">
      <div className="ui-popover-heading"><strong>Add a widget</strong><button type="button" className="icon-button ui-popover-action" aria-label="Close widget picker" popoverTarget={id} popoverTargetAction="hide"><Icon name="close" /></button></div>
      <div className="widget-picker-items">{widgetDefinitions().map((definition) => {
        const Items = definition.pickerItems ?? PickerItem;
        return <Items key={definition.kind} definition={definition} runtime={runtime} track={track} active={open} disabled={disabled} onAdd={(kind, initial) => {
          menu.close(); onAdd(kind, initial);
        }} />;
      })}</div>
    </Popover>
  </>;
}
