/** Synthetic game windows for native keyboard and ownership assertions in a real browser. */
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import GameWindow, { SceneView } from './GameWindow';
import type { DashboardClient } from '../dashboard/client';
import type { Scene } from '../generated/dashboard';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 8; i++) await frame(); }

const LIST = 'w.list';
type Rect = [number, number, number, number];
/** Five rows of 30 in a list box 100 high: the fourth overflows it and the fifth lies below it. */
const scene: Scene = {
  name: 'synthetic_window', size: [240, 200], viewport: [1920, 1080], textures: [], clips: [[10, 10, 210, 110]], quads: [],
  hovers: [0, 1, 2, 3, 4].map(index => ({ rect: [10, 10 + 30 * index, 200, 30] as Rect, clip: 0, list: LIST,
    title: `Policy ${index}`, details: index === 1 ? ['Not available: its availability condition does not hold', 'Attached to this organisation'] : [] })),
  lists: [{ path: LIST, rect: [10, 10, 200, 100], offset: 0, max: 300, horizontal: false, scale: 1 }], effects: [], notes: [],
};

/** The five rows as the service lays them out at the list's `offset`, down or across a box of
 * 100 units, in a container drawn at `scale`: each row 30 units along the list's axis, the
 * first at 10 pixels plus its place less the offset, in window pixels. */
function laidOut(horizontal: boolean, offset: number, scale: number): Scene {
  const along = (index: number) => 10 + (30 * index - offset) * scale;
  const [row, box] = [30 * scale, 100 * scale];
  return {
    name: 'synthetic_window', size: [240, 200], viewport: [1920, 1080], textures: [], quads: [],
    clips: [horizontal ? [10, 10, 10 + box, 10 + box] : [10, 10, 210, 10 + box]],
    hovers: [0, 1, 2, 3, 4].map(index => ({ rect: (horizontal ? [along(index), 10, row, box] : [10, along(index), 200, row]) as Rect,
      clip: 0, list: LIST, title: `Policy ${index}`, details: [] })),
    lists: [{ path: LIST, rect: horizontal ? [10, 10, box, box] : [10, 10, 200, box], offset, max: 300, horizontal, scale }], effects: [], notes: [],
  };
}

/** The list offsets the scrolling window's replies were laid out at, in order. */
let replied: number[] = [];

/** A client answering each request with the window laid out at the offset it asks for. */
function scrolling(horizontal: boolean, scale: number): DashboardClient {
  return { product: (request: { sourceId: string; revision: string; product: string; params: { scroll?: { path: string; offset: number }[] } }) => {
    const offset = request.params.scroll?.find(scroll => scroll.path === LIST)?.offset ?? 0;
    replied.push(offset);
    const data = { warnings: [], session: 'session-scrolling', title: 'Scrolling window', scene: laidOut(horizontal, offset, scale), refused: [], slots: [] };
    return Promise.resolve({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
  } } as unknown as DashboardClient;
}

/** A window whose one quad is its page times a colour, as two windows draw it at once. */
const coloured: Scene = {
  name: 'synthetic_window', size: [40, 40], viewport: [1920, 1080], textures: [{ id: 1, width: 1, height: 1 }], clips: [],
  quads: [{ texture: 1, corners: [[0, 0], [20, 0], [20, 20], [0, 20]], uv: [[0, 0], [1, 0], [1, 1], [0, 1]], color: [1, 0.5, 0.5, 1] }],
  hovers: [], lists: [], effects: [], notes: [],
};
/** One opaque texel. */
const PIXEL = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==';

const container = document.createElement('div');
document.body.append(container);
const root = createRoot(container);
let scrolls: [string, number][] = [];

/** Asks made of each client and the window it answers with. */
function client(name: string, asked: string[], window = scene): DashboardClient {
  return { product: (request: { sourceId: string; revision: string; product: string }) => {
    let data: unknown;
    if (request.product === 'game-window-artwork') data = { warnings: [], items: [{ id: 1, width: 1, height: 1, image: PIXEL }] };
    else {
      asked.push(name);
      data = { warnings: [], session: `session-${name}`, title: `Window of ${name}`, scene: window, refused: [], slots: [] };
    }
    return Promise.resolve({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
  } } as unknown as DashboardClient;
}

const asked: string[] = [];
const clients = { first: client('first', asked), second: client('second', asked), twin: client('twin', [], coloured) };

export const fixture = {
  async mountScene() {
    scrolls = [];
    flushSync(() => root.render(<SceneView scene={scene} title="Policies" images={new Map()} tints={new Map()} version={0} zoom={1}
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
      start: active ? parseFloat(horizontal ? active.style.left : active.style.top) : null };
  },
  /** Two windows of one client drawing the same coloured page at once. */
  async mountTwins() {
    const props = { client: clients.twin, source: { sourceId: 's', revision: 'r' }, branchId: '1', hour: 10,
      target: { kind: 'mioPolicies', organisation: '7' } as const };
    flushSync(() => root.render(<>{[0, 1].map(index => <div key={index} data-twin={index}><GameWindow {...props} /></div>)}</>));
    await settle();
  },
  /** Each twin's drawn quads, once both draw or a second has passed. */
  async twinQuads() {
    const count = () => [0, 1].map(index => container.querySelectorAll(`[data-twin="${index}"] .game-window-quad`).length);
    for (let wait = 0; wait < 60 && count().some(quads => quads === 0); wait++) await new Promise(resolve => setTimeout(resolve, 20));
    return count();
  },
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
