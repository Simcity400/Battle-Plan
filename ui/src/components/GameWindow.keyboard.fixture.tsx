/** Synthetic game windows for native keyboard and ownership assertions in a real browser. */
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import GameWindow, { SceneView } from './GameWindow';
import type { DashboardClient } from '../dashboard/client';
import { contentRevision } from '../dashboard/contentRevision';
import type { Scene } from '../generated/dashboard';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 8; i++) await frame(); }

const LIST = 'w.list';
type Rect = [number, number, number, number];
/** Five rows of 30 in a list box 100 high: the fourth overflows it and the fifth lies below it. */
/** One opaque texel. */
const PIXEL = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==';
/** A distinct image a reply paints, named by `what`. */
const painted = (what: string) => `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" data-what="${what}"/>`)}`;

const scene: Scene = {
  name: 'synthetic_window', size: [240, 200], viewport: [1920, 1080], image: PIXEL,
  hovers: [0, 1, 2, 3, 4].map(index => ({ rect: [10, 10 + 30 * index, 200, 30] as Rect, clip: [10, 10, 210, 110] as Rect, list: LIST,
    title: `Policy ${index}`, details: index === 1 ? ['Not available: its availability condition does not hold', 'Attached to this organisation'] : [] })),
  lists: [{ path: LIST, rect: [10, 10, 200, 100], offset: 0, max: 300, horizontal: false, scale: 1 }], notes: [],
};

/** The five rows as the service lays them out at the list's `offset`, down or across a box of
 * 100 units, in a container drawn at `scale`: each row 30 units along the list's axis, the
 * first at 10 pixels plus its place less the offset, in window pixels. */
function laidOut(horizontal: boolean, offset: number, scale: number): Scene {
  const along = (index: number) => 10 + (30 * index - offset) * scale;
  const [row, box] = [30 * scale, 100 * scale];
  const clip = (horizontal ? [10, 10, 10 + box, 10 + box] : [10, 10, 210, 10 + box]) as Rect;
  return {
    name: 'synthetic_window', size: [240, 200], viewport: [1920, 1080], image: painted(`offset ${offset}`),
    hovers: [0, 1, 2, 3, 4].map(index => ({ rect: (horizontal ? [along(index), 10, row, box] : [10, along(index), 200, row]) as Rect,
      clip, list: LIST, title: `Policy ${index}`, details: [] })),
    lists: [{ path: LIST, rect: horizontal ? [10, 10, box, box] : [10, 10, 200, box], offset, max: 300, horizontal, scale }], notes: [],
  };
}

/** The list offsets the scrolling window's replies were laid out at, in order. */
let replied: number[] = [];

/** A client answering each request with the window laid out at the offset it asks for. */
function scrolling(horizontal: boolean, scale: number): DashboardClient {
  return { product: (request: { sourceId: string; revision: string; product: string; params: { scroll?: { path: string; offset: number }[] } }) => {
    const offset = request.params.scroll?.find(scroll => scroll.path === LIST)?.offset ?? 0;
    replied.push(offset);
    const data = { warnings: [], title: 'Scrolling window', scene: laidOut(horizontal, offset, scale), refused: [], slots: [] };
    return Promise.resolve({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
  } } as unknown as DashboardClient;
}

/** A small window two windows of one client draw at once. */
const twin: Scene = { name: 'synthetic_window', size: [40, 40], viewport: [1920, 1080], image: PIXEL, hovers: [], lists: [], notes: [] };

const container = document.createElement('div');
document.body.append(container);
const root = createRoot(container);
let scrolls: [string, number][] = [];

/** Asks made of each client and the window it answers with. */
function client(name: string, asked: string[], window = scene): DashboardClient {
  return { product: (request: { sourceId: string; revision: string; product: string }) => {
    asked.push(name);
    const data = { warnings: [], title: `Window of ${name}`, scene: window, refused: [], slots: [] };
    return Promise.resolve({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
  } } as unknown as DashboardClient;
}

const asked: string[] = [];
const clients = { first: client('first', asked), second: client('second', asked), twin: client('twin', [], twin) };

/** A client painting each reply for the hour it was asked at and its count of replies so far. */
let painting = 0;
const following = { product: (request: { sourceId: string; revision: string; product: string; params: { hour: number } }) => {
  const image = painted(`hour ${request.params.hour} reply ${++painting}`);
  const data = { warnings: [], title: 'Following window', scene: { ...twin, image }, refused: [], slots: [] };
  return Promise.resolve({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
} } as unknown as DashboardClient;

/** The image a mounted window shows, once it is `what` or a second has passed. */
async function shownImage(what?: string) {
  const image = () => container.querySelector<HTMLImageElement>('.game-window-image');
  const named = () => decodeURIComponent(image()?.getAttribute('src') ?? '').match(/data-what="([^"]*)"/)?.[1] ?? null;
  for (let wait = 0; wait < 60 && what !== undefined && named() !== what; wait++) await new Promise(resolve => setTimeout(resolve, 20));
  return named();
}

export const fixture = {
  async mountScene() {
    scrolls = [];
    flushSync(() => root.render(<SceneView scene={scene} title="Policies" zoom={1}
      offsetOf={(_path, drawn) => drawn} onScroll={(path, offset) => { scrolls.push([path, offset]); }} />));
    await settle();
  },
  async mountWindow(which: keyof typeof clients) {
    flushSync(() => root.render(<GameWindow client={clients[which]} source={{ sourceId: 's', revision: 'r' }} branchId="1" hour={10}
      target={{ kind: 'mioPolicies', organisation: '7' }} />));
    await settle();
  },
  /** A window whose rows the client lays out at each offset asked for, down or across. */
  async mountScrolling(horizontal: boolean, scale: number) {
    replied = [];
    flushSync(() => root.render(<GameWindow client={scrolling(horizontal, scale)} source={{ sourceId: 's', revision: 'r' }} branchId="1" hour={10}
      target={{ kind: 'mioPolicies', organisation: '7' }} />));
    await settle();
  },
  /** The offsets replied at, once the last is `offset` or a second has passed, and where the
   * focused row is drawn: its start along the list's axis. */
  async repliedAt(offset: number, horizontal: boolean) {
    for (let wait = 0; wait < 60 && replied[replied.length - 1] !== offset; wait++) await new Promise(resolve => setTimeout(resolve, 20));
    await settle();
    const active = document.activeElement as HTMLElement | null;
    return { replied: [...replied], active: active?.getAttribute('aria-label') ?? null,
      start: active ? parseFloat(horizontal ? active.style.left : active.style.top) : null, image: await shownImage(`offset ${offset}`) };
  },
  /** Two windows of one client drawn at once. */
  async mountTwins() {
    const props = { client: clients.twin, source: { sourceId: 's', revision: 'r' }, branchId: '1', hour: 10,
      target: { kind: 'mioPolicies', organisation: '7' } as const };
    flushSync(() => root.render(<>{[0, 1].map(index => <div key={index} data-twin={index}><GameWindow {...props} /></div>)}</>));
    await settle();
  },
  /** Each twin's drawn images, once both draw or a second has passed. */
  async twinImages() {
    const count = () => [0, 1].map(index => container.querySelectorAll(`[data-twin="${index}"] .game-window-image`).length);
    for (let wait = 0; wait < 60 && count().some(quads => quads === 0); wait++) await new Promise(resolve => setTimeout(resolve, 20));
    return count();
  },
  /** A window of the following client at `hour`. */
  async mountFollowing(hour: number) {
    flushSync(() => root.render(<GameWindow client={following} source={{ sourceId: 's', revision: 'r' }} branchId="1" hour={hour}
      target={{ kind: 'mioPolicies', organisation: '7' }} />));
    await settle();
  },
  /** The service's presentation generation advances: content-dependent windows are asked again. */
  async advancePresentation() {
    const revision = contentRevision(following);
    revision.observePresentation(1);
    revision.observePresentation(2);
    await settle();
  },
  shownImage,
  focusFirstOutside() { (document.activeElement as HTMLElement | null)?.blur(); document.body.focus(); },
  snapshot() {
    const active = document.activeElement as HTMLElement | null;
    const clip = container.querySelector('.is-hovers .game-window-clip'), box = container.querySelector('.game-window-scene');
    return { active: active?.getAttribute('aria-label') ?? null, scrolls, asked: [...asked],
      scene: box?.getAttribute('aria-label') ?? null, scrolled: [clip?.scrollTop ?? 0, box?.scrollTop ?? 0] };
  },
  settle,
};

(window as unknown as { gameWindowFixture: typeof fixture }).gameWindowFixture = fixture;
