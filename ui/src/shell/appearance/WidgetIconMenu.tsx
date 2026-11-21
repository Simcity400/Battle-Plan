import { createContext, useMemo } from 'react';
import { widgetDefinition } from '../../dashboard/definitions';
import type { WidgetKind } from '../../dashboard/model';
import { useIconStyleMenu } from './IconStyleMenu';
import { iconStyleOf } from './iconStyle';
import type { IconStyle } from './iconStyle';
import { defaultSidebarIcon, sidebarIconIds } from './sidebarIcons';
import type { Appearance } from './useAppearance';

export interface WidgetIcons {
  style: (kind: WidgetKind) => IconStyle;
  openMenu: (kind: WidgetKind, anchor: HTMLElement, title: string, x: number, y: number) => void;
  edit: (kind: WidgetKind, anchor: HTMLElement, title: string, part: 'icon' | 'colour', focusAfter?: HTMLElement | null) => void;
  reset: (kind: WidgetKind) => void;
}

export const settingsButtonOf = (element: Element) => element.closest('article')?.querySelector<HTMLElement>('[data-widget-settings]') ?? null;

const unstyled: WidgetIcons = { style: () => ({}), openMenu() {}, edit() {}, reset() {} };
export const WidgetIconsContext = createContext<WidgetIcons>(unstyled);

export function useWidgetIconMenu(appearance: Appearance) {
  const mode = appearance.resolved.mode;
  const muted = appearance.resolved.colors.muted;
  const menu = useIconStyleMenu<WidgetKind>(appearance, {
    label: 'Widget icon',
    read: settings => settings.widgetIcons,
    write: (settings, widgetIcons) => ({ ...settings, widgetIcons }),
    defaultIcon: kind => widgetDefinition(kind)?.icon ?? 'grid',
    defaultColour: () => muted,
    swatches: [...new Set([muted, ...sidebarIconIds.map(id => defaultSidebarIcon(id)[mode])])],
  });
  const styles = appearance.settings.widgetIcons;
  const { items, resets, edit, change, open } = menu;
  const icons = useMemo<WidgetIcons>(() => ({
    style: kind => iconStyleOf(styles, kind),
    openMenu: (kind, anchor, title, x, y) => open(x, y, [...items(kind, anchor, title, settingsButtonOf(anchor)), ...resets(kind)]),
    edit,
    reset: kind => change(kind, { icon: null, colour: null }),
  }), [styles, items, resets, edit, change, open]);
  return { icons, menu: menu.menu };
}
