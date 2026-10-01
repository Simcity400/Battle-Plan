import { oklchToRgb, parseColor, rgbToOklch, toHex } from '../shell/appearance/color.ts';
import type { Rgb } from '../shell/appearance/color';
import { categoricalPalette, distance } from './palette.ts';

/** How far apart (OKLab ×100) two factions' default colours must be to be told apart;
 * only closer pairs clash, and the later one is moved just this far. (A ΔE of about
 * 2 is barely visible; 8 reads as two colours side by side.) */
export const FACTION_APART = 8;

/** Every recorded faction's campaign colour: the user's choice when there is one,
 * otherwise a default. The default is the leader's map colour exactly as recorded,
 * grey included (the user's decision, over spec 2.1's "fixed away from grey"), where
 * the leader is known; else the first categorical palette colour, in order, that
 * clashes with no colour placed before it. Only a default that clashes (is closer than
 * `FACTION_APART`) with a user's choice or an earlier default is moved, by the smallest
 * change of lightness that tells them apart, its hue and chroma kept, so a grey stays
 * grey; every other default is exactly as recorded. Users' choices are placed first,
 * then leader colours, then palette colours, each in faction id order, so a recorded
 * leader colour never gives way to a palette pick. Stable: the same factions always get
 * the same colours. */
export function factionColours(
  factions: readonly { id: string; leaderColor?: Rgb | null }[],
  chosen: Readonly<Record<string, string | undefined>>,
  surface: string,
): Map<string, string> {
  const palette = categoricalPalette(surface).map(hex => parseColor(hex)!);
  // Faction ids are recorded integers: order them numerically so ids are stable.
  const ordered = [...factions].sort((a, b) => compareIds(a.id, b.id));
  const fixed = ordered.flatMap(faction => {
    const colour = chosen[faction.id] && parseColor(chosen[faction.id]);
    return colour ? [{ id: faction.id, colour }] : [];
  });
  const open = ordered.filter(faction => !fixed.some(item => item.id === faction.id));
  const placed: Rgb[] = fixed.map(item => item.colour);
  const result = new Map<string, string>();
  for (const item of fixed) result.set(item.id, toHex(item.colour));
  const place = (id: string, colour: Rgb) => { placed.push(colour); result.set(id, toHex(colour)); };
  for (const faction of open) if (faction.leaderColor) place(faction.id, apart(faction.leaderColor, placed));
  let next = 0;
  for (const faction of open) {
    if (faction.leaderColor) continue;
    // The next palette colour in order that clashes with none placed, else the next one, moved apart.
    let chosenAt = -1;
    for (let offset = 0; offset < palette.length && chosenAt < 0; offset++) {
      const candidate = palette[(next + offset) % palette.length];
      if (placed.every(other => distance(candidate, other) >= FACTION_APART)) chosenAt = (next + offset) % palette.length;
    }
    const at = chosenAt < 0 ? next % palette.length : chosenAt;
    next = at + 1;
    place(faction.id, apart(palette[at], placed));
  }
  return result;
}

/** `colour` unchanged when it is at least `FACTION_APART` from every colour `placed`;
 * else the nearest lightness step (alternating lighter and darker, hue and chroma
 * kept) that is, or failing that the one furthest from them. */
export function apart(colour: Rgb, placed: readonly Rgb[]): Rgb {
  const gap = (candidate: Rgb) => placed.reduce((least, other) => Math.min(least, distance(candidate, other)), Infinity);
  if (gap(colour) >= FACTION_APART) return colour;
  const start = rgbToOklch(colour);
  let best = { colour, gap: gap(colour) };
  for (let step = 1; step <= 90; step++) {
    const delta = (step % 2 ? 1 : -1) * Math.ceil(step / 2) * 0.01;
    const lightness = start.l + delta;
    if (lightness < 0.05 || lightness > 0.97) continue;
    // Checked as it will be shown: rounded to whole sRGB bytes.
    const candidate = parseColor(toHex(oklchToRgb({ ...start, l: lightness })))!;
    const apartBy = gap(candidate);
    if (apartBy >= FACTION_APART) return candidate;
    if (apartBy > best.gap) best = { colour: candidate, gap: apartBy };
  }
  return best.colour;
}

function compareIds(a: string, b: string): number {
  return a.length - b.length || (a < b ? -1 : a > b ? 1 : 0);
}
