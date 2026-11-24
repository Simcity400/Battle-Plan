import { oklchToRgb, parseColor, rgbToOklch, toHex } from '../shell/appearance/color.ts';
import type { Rgb } from '../shell/appearance/color';
import { colour, distance } from './palette.ts';

export const FACTION_APART = 8;

export function factionColours(
  factions: readonly { id: string; leaderColor?: Rgb | null }[],
  chosen: Readonly<Record<string, string | undefined>>,
  surface: string,
): Map<string, string> {
  const ordered = [...factions].sort((a, b) => compareIds(a.id, b.id));
  const fixed = ordered.flatMap(faction => {
    const picked = chosen[faction.id] && parseColor(chosen[faction.id]);
    return picked ? [{ id: faction.id, colour: picked }] : [];
  });
  const open = ordered.filter(faction => !fixed.some(item => item.id === faction.id));
  const placed: Rgb[] = fixed.map(item => item.colour);
  const result = new Map<string, string>();
  for (const item of fixed) result.set(item.id, toHex(item.colour));
  const place = (id: string, rgb: Rgb) => { placed.push(rgb); result.set(id, toHex(rgb)); };
  for (const faction of open) if (faction.leaderColor) place(faction.id, apart(faction.leaderColor, placed));
  let next = 0;
  for (const faction of open) {
    if (faction.leaderColor) continue;
    let picked: Rgb | null = null;
    for (let offset = 0; offset < 32; offset++) {
      const candidate = parseColor(colour(surface, next + offset))!;
      if (placed.every(other => distance(candidate, other) >= FACTION_APART)) {
        next = next + offset + 1;
        picked = candidate;
        break;
      }
    }
    const candidate = picked ?? parseColor(colour(surface, next++))!;
    place(faction.id, apart(candidate, placed));
  }
  return result;
}

export function apart(colour: Rgb, placed: readonly Rgb[]): Rgb {
  const gap = (candidate: Rgb) => placed.reduce((least, other) => Math.min(least, distance(candidate, other)), Infinity);
  if (gap(colour) >= FACTION_APART) return colour;
  const start = rgbToOklch(colour);
  let best = { colour, gap: gap(colour) };
  for (let step = 1; step <= 90; step++) {
    const delta = (step % 2 ? 1 : -1) * Math.ceil(step / 2) * 0.01;
    const lightness = start.l + delta;
    if (lightness < 0.05 || lightness > 0.97) continue;
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
