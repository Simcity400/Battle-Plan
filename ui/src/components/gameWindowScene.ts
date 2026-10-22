import type { Scene, SceneList } from '../generated/dashboard';

/** Interface pixels one wheel line scrolls when the pointer reports lines. */
export const WHEEL_LINE_PIXELS = 16;

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
