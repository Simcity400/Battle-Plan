import type { KeyboardEvent, MouseEvent } from 'react';
import type { IconName } from '../../components/Icons';
import { pageIcon } from '../../dashboard/pages';
import { settingsSections } from '../settingsMetadata';
import { useIconStyleMenu } from './IconStyleMenu';
import { defaultSidebarIcon, isSidebarIconId, sidebarIconIds } from './sidebarIcons';
import type { SidebarIconId } from './sidebarIcons';
import type { Appearance } from './useAppearance';

export function defaultSidebarGlyph(id: SidebarIconId): IconName {
  if (id === 'analytics') return 'chart-bars';
  if (id === 'map') return 'map';
  if (id === 'settings') return 'settings';
  return settingsSections.find(section => section.id === id)?.icon ?? pageIcon(id);
}

export function useSidebarIconMenu(appearance: Appearance) {
  const mode = appearance.resolved.mode;
  const muted = appearance.resolved.colors.muted;
  const menu = useIconStyleMenu<SidebarIconId>(appearance, {
    label: 'Sidebar icon',
    read: settings => settings.sidebarIcons,
    write: (settings, sidebarIcons) => ({ ...settings, sidebarIcons }),
    defaultIcon: defaultSidebarGlyph,
    defaultColour: () => muted,
    swatches: [...new Set([muted, ...sidebarIconIds.map(id => defaultSidebarIcon(id)[mode])])],
  });
  const items = (id: SidebarIconId, button: HTMLButtonElement, name: string = defaultSidebarIcon(id).label) => menu.items(id, button, name);
  const open = (event: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>) => {
    if (event.defaultPrevented) return;
    const button = event.target instanceof Element ? event.target.closest<HTMLButtonElement>('button[data-sidebar-icon]') : null;
    const id = button?.dataset.sidebarIcon;
    if (!button || !id || !isSidebarIconId(id)) return;
    event.preventDefault();
    button.focus({ preventScroll: true });
    const box = button.getBoundingClientRect();
    menu.open('clientX' in event ? event.clientX : box.left, 'clientY' in event ? event.clientY : box.bottom, items(id, button));
  };
  return {
    custom: (id: SidebarIconId) => menu.style(id).colour, icon: (id: SidebarIconId) => menu.style(id).icon, items,
    onContextMenu: open,
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
      if (!event.defaultPrevented && (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey)) open(event);
    },
    menu: menu.menu,
  };
}
