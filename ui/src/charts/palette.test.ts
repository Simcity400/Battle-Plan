import test from 'node:test';
import assert from 'node:assert/strict';
import { contrast, isDark, mixRgb, oklchToRgb, parseColor, rgbToOklch, solveLightness, toHex } from '../shell/appearance/color.ts';
import type { Rgb } from '../shell/appearance/color';
import { BUILT_IN_THEMES, themeColors } from '../shell/appearance/themes.ts';
import {
  reservedGrey,
  RESERVED_SEPARATION,
  authoredHex as authoredHexForGrey,
  readableColour as readableForGrey,
  CATEGORY_LIMIT,
  SECONDARY_DASH,
  authoredHex,
  categoricalPalette,
  colour,
  distance,
  GREY_CHROMA,
  lineStyle,
  lineStyles,
  MARK_CONTRAST,
  nudgeApart,
} from './palette.ts';
import { NearestColours } from './paletteDistance.ts';

function scalarDistance(first: Rgb, second: Rgb): number {
  const a = rgbToOklch(first), b = rgbToOklch(second);
  const ax = a.c * Math.cos(a.h * Math.PI / 180), ay = a.c * Math.sin(a.h * Math.PI / 180);
  const bx = b.c * Math.cos(b.h * Math.PI / 180), by = b.c * Math.sin(b.h * Math.PI / 180);
  return Math.hypot(a.l - b.l, ax - bx, ay - by) * 100;
}

function scalarNudge(colors: readonly Rgb[], background: Rgb, minDistance = 12, avoid: readonly Rgb[] = []): Rgb[] {
  const dark = isDark(background);
  const fallback = categoricalPalette(toHex(background)).map(hex => parseColor(hex)!);
  const result: Rgb[] = [];
  for (const [position, color] of colors.entries()) {
    let base = rgbToOklch(color);
    if (base.c < GREY_CHROMA) base = rgbToOklch(fallback[position % fallback.length]);
    let chosen = oklchToRgb(base);
    if (!(contrast(chosen, background) >= MARK_CONTRAST)) {
      chosen = oklchToRgb(solveLightness(rgbToOklch(chosen), background, MARK_CONTRAST, dark ? 'lighter' : 'darker'));
    }
    const mark = (candidate: Rgb) => contrast(candidate, background) >= MARK_CONTRAST && rgbToOklch(candidate).c >= GREY_CHROMA;
    const gap = (candidate: Rgb) => Math.min(...[...avoid, ...result].map(other => scalarDistance(candidate, other)));
    if (!(mark(chosen) && gap(chosen) >= minDistance)) {
      const start = rgbToOklch(chosen);
      let best: { color: Rgb; gap: number } | null = null;
      for (let step = 1; step <= 70; step++) {
        const delta = (step % 2 ? 1 : -1) * Math.ceil(step / 2) * 0.02 * (dark ? 1 : -1);
        const candidate = oklchToRgb({ ...start, c: Math.max(start.c, GREY_CHROMA * 2), l: Math.min(0.9, Math.max(0.2, start.l + delta)) });
        if (!mark(candidate)) continue;
        const apart = gap(candidate);
        if (apart >= minDistance) { best = { color: candidate, gap: apart }; break; }
        if (!best || apart > best.gap) best = { color: candidate, gap: apart };
      }
      if (best) chosen = best.color;
    }
    result.push(chosen);
  }
  return result;
}

test('the categorical palette reads on every built-in theme surface, in both modes', () => {
  for (const theme of BUILT_IN_THEMES) {
    for (const mode of ['dark', 'light'] as const) {
      const surface = themeColors(theme, mode).surface;
      const palette = categoricalPalette(surface);
      assert.equal(palette.length, CATEGORY_LIMIT);
      assert.equal(categoricalPalette(surface), palette, 'each surface is solved once');
      for (const color of palette) assert.ok(contrast(parseColor(color)!, parseColor(surface)!) >= MARK_CONTRAST - 0.05, `${theme.id}/${mode} ${color} on ${surface}`);
    }
  }
});

test('nudged colours are readable, never grey, apart from each other and stable', () => {
  const background = parseColor('#16181d')!;
  const input = ['#3a6ea5', '#3b6fa6', '#808080', '#101010'].map(hex => parseColor(hex)!);
  const first = nudgeApart(input, background);
  assert.deepEqual(nudgeApart(input, background), first);
  assert.deepEqual(nudgeApart(input.slice(0, 2), background), first.slice(0, 2));
  for (const color of first) {
    assert.ok(contrast(color, background) >= MARK_CONTRAST - 0.05);
    assert.ok(rgbToOklch(color).c >= GREY_CHROMA * 0.9);
  }
  assert.ok(distance(first[0], first[1]) >= 12);
});

test('indexed gaps preserve the scalar trial sequence, ties, contrast and exceptional inputs', () => {
  const input = ['#3a6ea5', '#3b6fa6', '#808080', '#101010', '#00a000', '#00a000', '#ffffff', '#e34948']
    .map(hex => parseColor(hex)!);
  for (const surface of ['#16181d', '#ffffff', '#777777']) {
    const background = parseColor(surface)!;
    for (const minimum of [0, 12, 100, Infinity, NaN, -1]) {
      for (const avoid of [[], input.slice(0, 3), [...input, input[0]]]) {
        assert.deepEqual(nudgeApart(input, background, minimum, avoid), scalarNudge(input, background, minimum, avoid));
      }
    }
  }
  const exceptional = [{ r: NaN, g: 0, b: 0 }, { r: Infinity, g: 0, b: 0 }, { r: -Infinity, g: 0, b: 0 }];
  for (const avoid of [exceptional, [...input, ...exceptional]]) {
    assert.deepEqual(nudgeApart([...input, ...exceptional], input[0], 12, avoid), scalarNudge([...input, ...exceptional], input[0], 12, avoid));
  }
});

test('indexed gaps retain empty, incremental, boundary and nonfinite scalar minima', () => {
  const colors: Rgb[] = [
    { r: 0, g: 0, b: 0 }, { r: 0, g: 0, b: -0 },
    { r: 58, g: 110, b: 165 }, { r: 58 + Number.EPSILON * 58, g: 110, b: 165 },
    { r: -1, g: 256, b: 128 }, { r: Number.MAX_VALUE, g: 0, b: 0 },
    { r: NaN, g: 0, b: 0 }, { r: Infinity, g: 0, b: 0 },
  ];
  const neighbours = new NearestColours();
  const added: Rgb[] = [];
  for (const color of colors) {
    for (const query of colors) {
      assert.equal(distance(query, color), scalarDistance(query, color));
      assert.equal(neighbours.gap(query), Math.min(...added.map(other => scalarDistance(query, other))));
    }
    neighbours.add(color);
    added.push(color);
  }
});

test('nearest gaps match every scalar operand at 5000 colours, with fewer comparisons', context => {
  const samples = Array.from({ length: 5000 }, (_, index): Rgb => ({
    r: 20 + (index % 25) * 8,
    g: 24 + (Math.floor(index / 25) % 20) * 10,
    b: 25 + Math.floor(index / 500) * 22,
  }));
  const dense = samples.map((_, index): Rgb => index % 5 === 0 ? samples[0] : ({
    r: 58 + (index % 50) / 500,
    g: 110 + (Math.floor(index / 50) % 50) / 500,
    b: 165 + Math.floor(index / 2500) / 500,
  }));
  const queries = Array.from({ length: 64 }, (_, index) => samples[(index * 79) % samples.length]);
  queries.push(...dense.filter((_, index) => index % 79 === 0));
  const hypot = Math.hypot;
  let comparisons = 0, counting = false;
  context.after(() => { Math.hypot = hypot; });
  Math.hypot = (...values: number[]) => {
    if (counting && values.length === 3) comparisons++;
    return hypot(...values);
  };
  for (const colors of [samples, dense]) {
    const expected = queries.map(query => colors.reduce((best, other) => Math.min(best, scalarDistance(query, other)), Infinity));
    const neighbours = new NearestColours(colors);
    comparisons = 0;
    counting = true;
    assert.deepEqual(queries.map(query => neighbours.gap(query)), expected);
    counting = false;
    assert.ok(comparisons < colors.length * queries.length / 2, `${comparisons} scalar distance comparisons`);
  }
});

test('all 5000 fixed country colours stay present without quadratic gap scans', context => {
  const background = parseColor('#16181d')!;
  const dense = Array.from({ length: 5000 }, (_, index): Rgb => ({
    r: 58 + (index % 50) / 500,
    g: 110 + (Math.floor(index / 50) % 50) / 500,
    b: 165 + Math.floor(index / 2500) / 500,
  }));
  const repeated = Array.from({ length: 5000 }, () => parseColor('#3a6ea5')!);
  const hypot = Math.hypot;
  let comparisons = 0;
  context.after(() => { Math.hypot = hypot; });
  Math.hypot = (...values: number[]) => {
    if (values.length === 3) comparisons++;
    return hypot(...values);
  };
  for (const [colors, minimum] of [[dense, 0], [repeated, 12]] as const) {
    comparisons = 0;
    const result = nudgeApart(colors, background, minimum);
    const used = comparisons;
    assert.equal(result.length, 5000);
    assert.deepEqual(result.slice(0, 16), scalarNudge(colors.slice(0, 16), background, minimum));
    assert.ok(used < 5000 * 1000, `${used} scalar distance comparisons`);
    for (const color of result) assert.ok(contrast(color, background) >= MARK_CONTRAST - 0.05);
  }
});

test('ordinal colours are stable and extend lazily', () => {
  const firstOnly = colour('#12151a', 0);
  const later = colour('#12151a', 80);
  assert.equal(colour('#12151a', 0), firstOnly);
  assert.notEqual(firstOnly, later);
});

test('ordinal palette stays unique and >= 3:1 up to 1,500 across every built-in theme, mode, and chip background', () => {
  const count = 1500;
  for (const theme of BUILT_IN_THEMES) {
    for (const mode of ['dark', 'light'] as const) {
      const tokens = themeColors(theme, mode);
      const surfaceRgb = parseColor(tokens.surface)!;
      const raisedRgb = parseColor(tokens.raised)!;
      const textRgb = parseColor(tokens.text)!;
      const accentRgb = parseColor(tokens.accent)!;
      const hoverChipRgb = mixRgb(surfaceRgb, textRgb, 0.08);
      const selectedChipRgb = mixRgb(surfaceRgb, accentRgb, 0.22);
      const backgrounds = [
        ['surface', surfaceRgb],
        ['raised', raisedRgb],
        ['hover', hoverChipRgb],
        ['selected', selectedChipRgb],
      ] as const;
      const extras = [tokens.raised, toHex(hoverChipRgb), toHex(selectedChipRgb)];
      const seen = new Set<string>();
      for (let i = 0; i < count; i++) {
        const hex = colour(tokens.surface, i, { contrastSurfaces: extras });
        assert.equal(seen.has(hex), false, `${theme.id}/${mode} duplicate ${hex} at ordinal ${i}`);
        seen.add(hex);
        const rgb = parseColor(hex)!;
        for (const [name, bg] of backgrounds) {
          const ratio = contrast(rgb, bg);
          assert.ok(ratio >= MARK_CONTRAST, `${theme.id}/${mode} ordinal ${i} (${hex}) contrast ${ratio.toFixed(4)} < 3.0 on ${name}`);
        }
      }
      assert.equal(seen.size, count);
    }
  }
  const midGreySeen = new Set<string>();
  for (let i = 0; i < count; i++) midGreySeen.add(colour('#808080', i));
  assert.equal(midGreySeen.size, count);
});

test('authored colours stay byte-exact and secondary lines pair dash patterns', () => {
  const surface = '#16181d';
  const exact = '#4f86f7';
  assert.equal(authoredHex(exact), exact);
  assert.equal(authoredHex([79, 134, 247]), '#4f86f7');
  assert.deepEqual(lineStyle(surface, 0), { color: colour(surface, 0), primary: [], secondary: SECONDARY_DASH });
  assert.ok(lineStyle(surface, CATEGORY_LIMIT).primary.length > 0);
  const styles = lineStyles([
    { key: 'a', preferred: 0 },
    { key: 'b', preferred: 1 },
    { key: 'c', preferred: CATEGORY_LIMIT },
  ], categoricalPalette(surface), surface);
  assert.equal(styles.get('a')!.color, colour(surface, 0));
  assert.deepEqual(styles.get('a')!.primary, []);
  assert.deepEqual(styles.get('a')!.secondary, SECONDARY_DASH);
  assert.equal(styles.get('c')!.color, colour(surface, CATEGORY_LIMIT));
  assert.ok(styles.get('c')!.primary.length > 0);
});



test('the reserved grey for contested and other moves away from a near-neutral identity colour in the same picture', () => {
  const surface = '#161616';
  const lightness = (hex: string) => rgbToOklch(parseColor(hex)!).l;
  const axis = readableForGrey(authoredHexForGrey([81, 78, 74]), surface);
  const ethiopia = readableForGrey(authoredHexForGrey([63, 90, 140]), surface);
  const plain = reservedGrey(surface);
  assert.ok(Math.abs(lightness(axis) - lightness(plain)) < RESERVED_SEPARATION, 'the recorded grey Axis reads like the usual reserved grey');
  const moved = reservedGrey(surface, [axis, ethiopia]);
  assert.notEqual(moved, plain);
  assert.ok(Math.abs(lightness(axis) - lightness(moved)) >= RESERVED_SEPARATION, 'beside a grey identity the reserved grey takes the darker step');
  assert.equal(reservedGrey(surface, [ethiopia]), plain, 'a chromatic identity leaves it where it was');
});
