import { useEffect, useState } from 'react';
import type { KeyboardEvent, MouseEvent } from 'react';
import type { MenuItem } from '../../components/ContextMenu';
import { useContextMenu } from '../../components/ContextMenu';
import { Icon } from '../../components/Icons';
import { Popover, usePopover } from '../../components/Popover';
import { ColorPicker } from './ColorField';
import { normalizeColor } from './color';
import { defaultSidebarIcon, isSidebarIconId, sidebarIconIds } from './sidebarIcons';
import type { SidebarIconId } from './sidebarIcons';
import type { Appearance } from './useAppearance';

/** One colour menu shared by the sidebar's navigation buttons. */
export function useSidebarIconMenu(appearance: Appearance) {
  const actions = useContextMenu('Sidebar icon actions');
  const picker = usePopover('start');
  const [editing, setEditing] = useState<SidebarIconId | null>(null);
  const [label, setLabel] = useState('');
  const [text, setText] = useState('');
  const mode = appearance.resolved.mode;
  const colour = (id: SidebarIconId) => appearance.settings.sidebarIcons[id] ?? defaultSidebarIcon(id)[mode];
  const value = editing ? colour(editing) : '';
  useEffect(() => setText(value), [value]);
  const setColour = (id: SidebarIconId, colour: string | null) => appearance.update(previous => {
    const sidebarIcons = { ...previous.sidebarIcons };
    if (colour) sidebarIcons[id] = colour;
    else delete sidebarIcons[id];
    return { ...previous, sidebarIcons };
  });
  const items = (id: SidebarIconId, button: HTMLButtonElement, name: string = defaultSidebarIcon(id).label): MenuItem[] => [
    { label: 'Change icon colour', icon: 'palette', run: () => {
      setEditing(id);
      setLabel(name);
      setText(colour(id));
      picker.trigger.current = button;
      picker.openAt(() => button.isConnected ? button.getBoundingClientRect() : null);
    } },
  ];
  const open = (event: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>) => {
    if (event.defaultPrevented) return;
    const button = event.target instanceof Element ? event.target.closest<HTMLButtonElement>('button[data-sidebar-icon]') : null;
    const id = button?.dataset.sidebarIcon;
    if (!button || !id || !isSidebarIconId(id)) return;
    event.preventDefault();
    button.focus({ preventScroll: true });
    const box = button.getBoundingClientRect();
    actions.open('clientX' in event ? event.clientX : box.left, 'clientY' in event ? event.clientY : box.bottom, items(id, button));
  };
  const commit = () => {
    const next = normalizeColor(text);
    if (next && editing) setColour(editing, next);
    else setText(value);
  };
  const menu = <>
    {actions.menu}
    <Popover menu={picker} className="color-picker sidebar-icon-picker" aria-label="Sidebar icon colour">
      {editing && <>
        <div className="ui-popover-heading"><strong>{label} icon</strong><div className="ui-popover-actions">
          {appearance.settings.sidebarIcons[editing] && <button type="button" className="icon-button ui-popover-action" aria-label="Reset icon colour"
            data-tooltip="Reset to default" onClick={() => setColour(editing, null)}><Icon name="refresh" /></button>}
          <button type="button" className="icon-button ui-popover-action" aria-label="Close icon colour picker" onClick={picker.close}><Icon name="close" /></button>
        </div></div>
        {picker.open && <ColorPicker value={value} onChange={colour => setColour(editing, colour)} swatches={[...new Set(sidebarIconIds.map(id => defaultSidebarIcon(id)[mode]))]} />}
        <label className="sidebar-icon-hex"><span>Hex</span><input data-popover-autofocus className="color-hex" aria-label="Icon hex value" value={text} spellCheck={false} maxLength={9}
          onChange={event => setText(event.currentTarget.value)} onBlur={commit} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); commit(); } }} /></label>
        {appearance.notice && <p className="dash-inline-warning" role="status">{appearance.notice}</p>}
      </>}
    </Popover>
  </>;
  return {
    colour, items,
    onContextMenu: open,
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
      if (!event.defaultPrevented && (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey)) open(event);
    },
    menu,
  };
}
