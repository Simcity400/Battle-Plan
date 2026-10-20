import type { CSSProperties } from 'react';
import type { Scene, SceneList, SceneQuad, SceneTexture } from '../generated/dashboard';

/** The texture pages one artwork request asks for. */
export const ARTWORK_BATCH = 6;
/** Interface pixels one wheel line scrolls when the pointer reports lines. */
export const WHEEL_LINE_PIXELS = 16;

const px = (value: number) => `${value === 0 ? 0 : value}px`;

/** Whether a quad's page is drawn multiplied by its colour: its own page then holds the
 * product ({@link tintPixels}). */
export const tinted = (quad: Pick<SceneQuad, 'color'>) => quad.color[0] !== 1 || quad.color[1] !== 1 || quad.color[2] !== 1;

/** A page multiplied by a colour, by page and colour. */
export const tintName = (quad: Pick<SceneQuad, 'texture' | 'color'>) => `${quad.texture}|${quad.color.slice(0, 3).join(',')}`;

/** Multiplies every pixel's straight (unpremultiplied) colour by `rgb`, its alpha unchanged:
 * what the game's draw makes of a page times a colour. */
export function tintPixels(data: Uint8ClampedArray, rgb: readonly [number, number, number] | readonly number[]): void {
  for (let index = 0; index < data.length; index += 4) {
    data[index] = Math.round(data[index] * rgb[0]);
    data[index + 1] = Math.round(data[index + 1] * rgb[1]);
    data[index + 2] = Math.round(data[index + 2] * rgb[2]);
  }
}

/** How one quad is drawn: an element the size of its texture rectangle in texels, showing
 * that part of its page (repeating where the coordinates leave it), transformed so its
 * corners land exactly on the quad's. `url` is the page, already multiplied by the quad's
 * colour where it is {@link tinted}; the colour's alpha is the element's opacity. Null when
 * the quad covers no area. */
export function quadStyle(quad: SceneQuad, texture: Pick<SceneTexture, 'width' | 'height'>, url: string): CSSProperties | null {
  const W = texture.width, H = texture.height;
  const [[u0, v0], , [u1, v1]] = quad.uv;
  const dx = (u1 - u0) * W, dy = (v1 - v0) * H;
  if (!W || !H || !dx || !dy || !Number.isFinite(dx) || !Number.isFinite(dy)) return null;
  // Solved from the top-left, top-right and bottom-left corners.
  const [tl, tr, , bl] = quad.corners;
  const across = [tr[0] - tl[0], tr[1] - tl[1]], down = [bl[0] - tl[0], bl[1] - tl[1]];
  if (!(across[0] * down[1] - across[1] * down[0])) return null;
  const umin = Math.min(u0, u1), vmin = Math.min(v0, v1);
  const x0 = (u0 - umin) * W, y0 = (v0 - vmin) * H;
  const a = across[0] / dx, b = across[1] / dx, c = down[0] / dy, d = down[1] / dy;
  const e = tl[0] - a * x0 - c * y0, f = tl[1] - b * x0 - d * y0;
  const image = `url("${url}")`, size = `${px(W)} ${px(H)}`, position = `${px(-umin * W)} ${px(-vmin * H)}`;
  const style: CSSProperties = {
    width: px(Math.abs(dx)), height: px(Math.abs(dy)),
    backgroundImage: image, backgroundSize: size, backgroundRepeat: 'repeat', backgroundPosition: position,
    transformOrigin: '0 0', transform: `matrix(${a}, ${b}, ${c}, ${d}, ${e}, ${f})`,
  };
  const alpha = quad.color[3];
  if (alpha < 1) style.opacity = Math.max(0, alpha);
  return style;
}

/** Consecutive quads sharing one clip and whether they are muted, in draw order. `start` is
 * the first quad's index. A muted run is muted as one group, so its overlapping quads blend
 * with each other first, as one row would. */
export interface ClipRun { clip: number | undefined; muted: boolean; start: number; quads: SceneQuad[] }

export function clipRuns(quads: readonly SceneQuad[]): ClipRun[] {
  const runs: ClipRun[] = [];
  quads.forEach((quad, index) => {
    const last = runs[runs.length - 1], muted = quad.muted === true;
    if (last && last.clip === quad.clip && last.muted === muted) last.quads.push(quad);
    else runs.push({ clip: quad.clip, muted, start: index, quads: [quad] });
  });
  return runs;
}

/** A wheel movement in CSS pixels, whatever unit the pointer reported. */
export function wheelPixels(deltaY: number, deltaMode: number, page: number): number {
  return deltaMode === 1 ? deltaY * WHEEL_LINE_PIXELS : deltaMode === 2 ? deltaY * page : deltaY;
}

/** A list's offset after the wheel moved `pixels` CSS pixels over a window drawn at `zoom`. */
export function scrolledOffset(offset: number, pixels: number, zoom: number, max: number): number {
  return Math.min(Math.max(0, max), Math.max(0, offset + Math.round(pixels / (zoom > 0 ? zoom : 1))));
}

/** The smallest zoom a window is drawn at: below it the game's bitmap text is no longer
 * legible, so a window that does not fit at it scrolls in its space instead. */
export const MIN_ZOOM = 0.6;

/** The zoom that fits a whole window of `size` interface pixels into `available` CSS pixels,
 * never below {@link MIN_ZOOM}. */
export function fitZoom(size: readonly [number, number], available: readonly [number, number]): number {
  const [w, h] = size, [aw, ah] = available;
  if (w <= 0 || h <= 0 || aw <= 0 || ah <= 0) return 1;
  return Math.max(MIN_ZOOM, Math.min(aw / w, ah / h));
}

/** The topmost list whose area (`x, y, width, height`) holds a window point. */
export function listAt(scene: Pick<Scene, 'lists'>, x: number, y: number) {
  for (let index = scene.lists.length - 1; index >= 0; index--) {
    const list = scene.lists[index];
    const [lx, ly, lw, lh] = list.rect;
    if (x >= lx && y >= ly && x < lx + lw && y < ly + lh) return list;
  }
  return null;
}

/** The texture pages the scene's quads sample that `have` lacks, each once, in first-drawn order. */
export function missingTextures(scene: Pick<Scene, 'quads'>, have: (id: number) => boolean): number[] {
  const seen = new Set<number>();
  for (const quad of scene.quads) if (!seen.has(quad.texture) && !have(quad.texture)) seen.add(quad.texture);
  return [...seen];
}

export function batches<T>(items: readonly T[], size = ARTWORK_BATCH): T[][] {
  const groups: T[][] = [];
  for (let start = 0; start < items.length; start += size) groups.push(items.slice(start, start + size));
  return groups;
}

/** A list's offset after a key over it: a line of `line` window pixels (down and up, or right
 * and left for a list that scrolls across), a page of the list's extent, or its start and end,
 * each in the list's offset units; null for any other key. */
export function keyedOffset(key: string, offset: number, list: Pick<SceneList, 'rect' | 'max' | 'horizontal' | 'scale'>, line: number): number | null {
  const page = units(list.rect[axis(list) + 2], list);
  line = units(line, list);
  const steps: Record<string, number> = list.horizontal
    ? { ArrowRight: line, ArrowLeft: -line, PageDown: page, PageUp: -page }
    : { ArrowDown: line, ArrowUp: -line, PageDown: page, PageUp: -page };
  const step = steps[key];
  const next = key === 'Home' ? 0 : key === 'End' ? list.max : step === undefined ? null : offset + step;
  return next === null ? null : Math.min(Math.max(0, list.max), Math.max(0, Math.round(next)));
}

/** The index of a rectangle's start and extent along a list's scroll axis. */
const axis = (list: Pick<SceneList, 'horizontal'>) => list.horizontal ? 0 : 1;

/** A distance in window pixels in a list's offset units: its rows' container may be scaled. */
const units = (pixels: number, list: Pick<SceneList, 'scale'>) => pixels / (list.scale > 0 ? list.scale : 1);

/** A list's offset that brings a row (`x, y, width, height` as drawn at `offset`) wholly into the
 * list's box along the list's scroll axis, moving no further than that; unchanged when it is
 * already within. */
export function revealedOffset(offset: number, list: Pick<SceneList, 'rect' | 'max' | 'horizontal' | 'scale'>, row: readonly number[]): number {
  const at = axis(list), start = row[at], end = row[at] + row[at + 2];
  const boxStart = list.rect[at], boxEnd = list.rect[at] + list.rect[at + 2];
  const next = start < boxStart ? offset - units(boxStart - start, list) : end > boxEnd ? offset + units(end - boxEnd, list) : offset;
  return Math.min(Math.max(0, list.max), Math.max(0, Math.round(next)));
}

/** What a hover area says to assistive technology: its name and every detail the tooltip shows. */
export const described = (hover: { title: string; details: readonly string[] }) => [hover.title, ...hover.details].join('. ');
