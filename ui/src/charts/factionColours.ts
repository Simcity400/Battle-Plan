import { parseColor, toHex } from '../shell/appearance/color.ts';
import type { Rgb } from '../shell/appearance/color';
import { categoricalPalette, nudgeApart } from './palette.ts';

/** Every recorded faction's campaign colour: the user's choice when there is one,
 * otherwise a default. The default is the leader's map colour where the leader is
 * known, else the categorical palette in faction order. Defaults are made readable
 * on the theme, never grey (grey means "contested" or "no data") and kept apart
 * from each other and from the user's choices. Stable: the same factions always
 * get the same colours. */
export function factionColours(
  factions: readonly { id: string; leaderColor?: Rgb | null }[],
  chosen: Readonly<Record<string, string | undefined>>,
  surface: string,
): Map<string, string> {
  const background = parseColor(surface) ?? { r: 0, g: 0, b: 0 };
  const palette = categoricalPalette(surface).map(hex => parseColor(hex)!);
  // Faction ids are recorded integers: order them numerically so ids are stable.
  const ordered = [...factions].sort((a, b) => compareIds(a.id, b.id));
  const fixed = ordered.flatMap(faction => {
    const colour = chosen[faction.id] && parseColor(chosen[faction.id]);
    return colour ? [{ id: faction.id, colour }] : [];
  });
  const open = ordered.filter(faction => !fixed.some(item => item.id === faction.id));
  const wanted = open.map((faction, index) => faction.leaderColor ?? palette[index % palette.length]);
  // Defaults move away from the chosen colours as shown, which are never changed.
  const nudged = nudgeApart(wanted, background, 12, fixed.map(item => item.colour));
  const result = new Map<string, string>();
  for (const item of fixed) result.set(item.id, toHex(item.colour));
  open.forEach((faction, index) => result.set(faction.id, toHex(nudged[index])));
  return result;
}

function compareIds(a: string, b: string): number {
  return a.length - b.length || (a < b ? -1 : a > b ? 1 : 0);
}
