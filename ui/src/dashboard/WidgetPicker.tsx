import { Popover, usePopover } from '../components/Popover';
import { useId } from 'react';
import { Icon } from '../components/Icons';
import { widgetDefinitions } from './definitions';
import type { WidgetDefinition, WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { useProduct } from './products';
import { availabilityReason, availabilityRequest, validateWidgetAvailability } from './widgetAvailability';

function PickerItem({ definition, runtime, track, active, disabled, onAdd }: {
  definition: WidgetDefinition; runtime: WidgetRuntime; track: BoundTrack | undefined;
  active: boolean; disabled: boolean; onAdd: (kind: string) => void;
}) {
  const { data, error } = useProduct(runtime.client, availabilityRequest(definition, track), active, validateWidgetAvailability);
  const reason = availabilityReason(!!definition.availability, !!track, data, error);
  return <button type="button" disabled={disabled || reason !== null} data-tooltip={reason ?? undefined} onClick={() => onAdd(definition.kind)}>
    <span className="dash-widget-icon" aria-hidden="true"><Icon name={definition.icon} /></span>
    <span><strong>{definition.title}</strong><small>{reason ?? definition.description}</small></span><Icon name="plus" />
  </button>;
}

export default function WidgetPicker({ disabled, onAdd, onOpen, runtime }: { disabled: boolean; onAdd: (kind: string) => void; onOpen: () => void; runtime: WidgetRuntime }) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const track = runtime.tracks.find(item => item.id === runtime.campaignTracks[0]?.id);

  return <>
    <button ref={trigger} type="button" className="icon-button header-add-widget" aria-label="Add widget" data-tooltip="Add a widget" disabled={disabled}
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open} onClick={onOpen}><Icon name="plus" /></button>
    <Popover menu={menu} id={id} className="widget-picker" aria-label="Add a widget">
      <div className="ui-popover-heading"><strong>Add a widget</strong><button type="button" className="icon-button ui-popover-action" aria-label="Close widget picker" popoverTarget={id} popoverTargetAction="hide"><Icon name="close" /></button></div>
      <div className="widget-picker-items">{widgetDefinitions().map((definition) => <PickerItem key={definition.kind} definition={definition} runtime={runtime} track={track} active={open} disabled={disabled} onAdd={(kind) => {
        menu.close(); onAdd(kind);
      }} />)}</div>
    </Popover>
  </>;
}
