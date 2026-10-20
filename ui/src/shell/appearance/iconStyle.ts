// A sidebar item's or a widget kind's own icon and colour, kept with the appearance
// preferences. Either part left out shows the default.
import { isIconName } from '../../components/iconNames.ts';
import type { IconName } from '../../components/iconNames.ts';
import type { WidgetKind } from '../../dashboard/model.ts';
import { normalizeColor } from './color.ts';

/** A picture the user chose, drawn into a small square PNG and kept as a data URL. */
export type IconImage = `data:image/${string}`;
export interface IconStyle { colour?: string; icon?: IconName | IconImage }
export type IconStyles<Id extends string> = Partial<Record<Id, IconStyle>>;
export type WidgetIconStyles = IconStyles<WidgetKind>;

export const isIconImage = (value: unknown): value is IconImage =>
  typeof value === 'string' && /^data:image\/(?:png|jpeg|webp|gif);base64,[A-Za-z0-9+/]+={0,2}$/.test(value);

/** Widget kinds as layouts store them; any installed kind may be styled, so the registry is not consulted. */
export const isWidgetKind = (value: string): value is WidgetKind =>
  /^[a-zA-Z0-9_-]{1,80}$/.test(value) && !['__proto__', 'constructor', 'prototype'].includes(value);

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

/** A style's readable parts, or null when neither part is. */
export function parseIconStyle(input: unknown): IconStyle | null {
  if (!object(input)) return null;
  const colour = normalizeColor(input.colour);
  const icon = isIconName(input.icon) || isIconImage(input.icon) ? input.icon : null;
  return colour || icon ? { ...(colour ? { colour } : {}), ...(icon ? { icon } : {}) } : null;
}

/** Keeps the styles whose id is known and that have a readable part. */
export function parseIconStyles<Id extends string>(input: unknown, isId: (id: string) => id is Id): IconStyles<Id> {
  if (!object(input)) return {};
  return Object.fromEntries(Object.entries(input).flatMap(([id, value]) => {
    const style = isId(id) ? parseIconStyle(value) : null;
    return style ? [[id, style]] : [];
  })) as IconStyles<Id>;
}

/** An id's own style; ids are looked up as own entries only, so no id reads the object's prototype. */
export const iconStyleOf = <Id extends string>(styles: IconStyles<Id>, id: Id): IconStyle => (Object.hasOwn(styles, id) ? styles[id] : undefined) ?? {};

/** The styles with one id's parts changed; null returns a part to its default, and an id left with neither part is removed. */
export function withIconStyle<Id extends string>(styles: IconStyles<Id>, id: Id, change: { [Part in keyof IconStyle]?: IconStyle[Part] | null }): IconStyles<Id> {
  const style: IconStyle = { ...iconStyleOf(styles, id) };
  for (const part of ['colour', 'icon'] as const) {
    if (change[part] === null) delete style[part];
    else if (change[part] !== undefined) (style as Record<string, string>)[part] = change[part];
  }
  const next = { ...styles };
  if (style.colour || style.icon) next[id] = style;
  else delete next[id];
  return next;
}
