import { createContext, useMemo } from 'react';
import { widgetDefinition } from '../../dashboard/definitions';
import type { WidgetKind } from '../../dashboard/model';
import { useIconStyleMenu } from './IconStyleMenu';
import { iconStyleOf } from './iconStyle';
import type { IconStyle } from './iconStyle';
import { defaultSidebarIcon, sidebarIconIds } from './sidebarIcons';
import type { Appearance } from './useAppearance';

/** Each widget kind's header icon, as every copy of the kind draws it, and the menus that change it. */
export interface WidgetIcons {
  style: (kind: WidgetKind) => IconStyle;
  /** The header icon's right-click menu at a point: Change icon, Change icon colour and a reset for each part changed. */
  openMenu: (kind: WidgetKind, anchor: HTMLElement, title: string, x: number, y: number) => void;
  /** `focusAfter` takes focus when the picker closes (the gear menu names the widget's settings button). */
  edit: (kind: WidgetKind, anchor: HTMLElement, title: string, part: 'icon' | 'colour', focusAfter?: HTMLElement | null) => void;
  reset: (kind: WidgetKind) => void;
}

/** The settings button of the widget an element sits in, where its icon pickers return focus. */
export const settingsButtonOf = (element: Element) => element.closest('article')?.querySelector<HTMLElement>('[data-widget-settings]') ?? null;

const unstyled: WidgetIcons = { style: () => ({}), openMenu() {}, edit() {}, reset() {} };
/** Widgets drawn without the app (component tests, the recorder) show their default icons. */
export const WidgetIconsContext = createContext<WidgetIcons>(unstyled);

export function useWidgetIconMenu(appearance: Appearance) {
  const mode = appearance.resolved.mode;
  // Headers draw their icon in the muted text colour unless the kind has its own.
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
    // The header icon cannot hold focus, so a picker opened from it hands focus back to the widget's settings button.
    openMenu: (kind, anchor, title, x, y) => open(x, y, [...items(kind, anchor, title, settingsButtonOf(anchor)), ...resets(kind)]),
    edit,
    reset: kind => change(kind, { icon: null, colour: null }),
  }), [styles, items, resets, edit, change, open]);
  return { icons, menu: menu.menu };
}
