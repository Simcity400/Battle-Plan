import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, useSyncExternalStore } from 'react';
import type { CSSProperties, KeyboardEvent, ReactNode, RefObject } from 'react';
import { validateProductReply } from '../dashboard/client';
import type { DashboardClient, ProductRequest } from '../dashboard/client';
import { useContentRevision } from '../dashboard/contentRevision';
import WidgetState from './WidgetState';
import { pageCache } from './gameWindowPages';
import { WHEEL_LINE_PIXELS, batches, clipRuns, described, fitZoom, keyedOffset, listAt, missingTextures, quadStyle, revealedOffset, scrolledOffset, tintName,
  tintPixels, tinted, wheelPixels } from './gameWindowScene';
import type { ClipRun } from './gameWindowScene';
import type { GameWindowArtworkParams, GameWindowArtworkReply, GameWindowParams, GameWindowReply, GameWindowTarget, ListScroll, Scene,
  SceneHover, SceneTexture, WindowTextureImage } from '../generated/dashboard';
import './GameWindow.css';

/** How long the wheel rests before a scrolled list's window is asked for again. */
const SCROLL_SETTLE_MS = 60;
const NO_LINES: string[] = [];
const NO_SCROLL: ListScroll[] = [];

/** Each client's own number, so what one client asked for is never drawn for another. */
const clients = new WeakMap<DashboardClient, number>();
let nextClient = 0;
const clientNumber = (client: DashboardClient) => {
  let number = clients.get(client);
  if (number === undefined) clients.set(client, number = ++nextClient);
  return number;
};

const message = (error: unknown) => error instanceof Error ? error.message : String(error);

export interface GameWindowProps {
  client: DashboardClient;
  source: { sourceId: string; revision: string };
  branchId: string;
  hour: number;
  target: GameWindowTarget;
  onDiagnostics?: (lines: string[]) => void;
  /** The window's own title, once a reply names it. */
  onTitle?: (title: string | null) => void;
  /** False while hidden (a closed popover): nothing is asked for, and what was asking stops. */
  active?: boolean;
}

/** One game window, laid out by the service at the hour shown and drawn from its texture
 * pages. It is fitted whole into the space it has by zoom, never laid out again here:
 * scrolling a list asks the service for the window at the new offset. */
export default function GameWindow({ client, source, branchId, hour, target, onDiagnostics, onTitle, active = true }: GameWindowProps) {
  const { sourceId, revision } = source;
  // The window belongs to its client, the content folders it was read with, its recording and
  // its target: a change of any asks for it afresh and hides what was drawn meanwhile.
  const content = useContentRevision(client);
  const identity = JSON.stringify([clientNumber(client), content, sourceId, revision, branchId, target]);
  const [scroll, setScroll] = useState<{ identity: string; lists: ListScroll[] }>({ identity, lists: NO_SCROLL });
  const lists = scroll.identity === identity ? scroll.lists : NO_SCROLL;
  const key = JSON.stringify([identity, hour, lists]);
  const [shown, setShown] = useState<{ identity: string; key: string; reply: GameWindowReply } | null>(null);
  const [failure, setFailure] = useState<{ identity: string; detail: string } | null>(null);
  const [attempt, retry] = useReducer((count: number) => count + 1, 0);
  const shownKey = useRef<string | null>(null);
  shownKey.current = shown?.key ?? null;

  useEffect(() => {
    // Reopened on the window already drawn: nothing to ask.
    if (!active || shownKey.current === key) return;
    const controller = new AbortController();
    const params: GameWindowParams = { branchId, hour, window: target, ...(lists.length ? { scroll: lists } : {}) };
    const request: ProductRequest<GameWindowParams> = { sourceId, revision, product: 'game-window', params };
    client.product<GameWindowReply>(request, controller.signal).then(reply => {
      if (controller.signal.aborted) return;
      setShown({ identity, key, reply: validateProductReply(reply, request).data });
      setFailure(null);
    }).catch((error: unknown) => {
      if (!controller.signal.aborted) setFailure({ identity, detail: message(error) });
    });
    return () => controller.abort();
  // The target and lists are read through `identity` and `key`.
  }, [client, sourceId, revision, branchId, hour, identity, key, active, attempt]); // eslint-disable-line react-hooks/exhaustive-deps

  // A reply of another window or recording is never shown; one of an earlier hour or scroll is, until its successor arrives.
  const reply = shown?.identity === identity ? shown.reply : null;
  const scene = reply?.scene ?? null;
  const owner = reply ? `${clientNumber(client)}:${content}:${reply.session}` : null;
  const artwork = useArtwork(client, sourceId, revision, owner, reply?.session ?? null, scene, active);

  const title = reply?.title ?? null;
  useEffect(() => { onTitle?.(title); }, [onTitle, title]);
  const failed = failure?.identity === identity ? failure.detail : null;
  useEffect(() => {
    if (!onDiagnostics) return;
    // Hidden, it reports nothing; what it reported belonged to what was shown.
    const lines = !active ? NO_LINES : [...reply?.warnings ?? [], ...reply?.refused ?? [], ...scene?.notes ?? [], ...scene?.effects ?? [],
      ...(reply?.slots ?? []).filter(slot => slot.state !== 'bound').map(slot => `${slot.name}: ${slot.detail ?? (slot.state === 'refused' ? 'refused' : 'not filled')}`),
      ...artwork.warnings, ...reply && failed ? [`Couldn’t load the window again: ${failed}`] : []];
    onDiagnostics(lines.length ? lines : NO_LINES);
  }, [onDiagnostics, active, reply, scene, artwork.warnings, failed]);
  useEffect(() => () => onDiagnostics?.(NO_LINES), [onDiagnostics]);

  // The offsets the wheel has reached, ahead of the scene drawn at the last settled ones.
  const pending = useRef<{ identity: string; offsets: Map<string, number> }>({ identity, offsets: new Map() });
  if (pending.current.identity !== identity) pending.current = { identity, offsets: new Map() };
  const settle = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(settle.current), [identity, active]);
  const onScroll = useCallback((path: string, offset: number) => {
    const owner = pending.current;
    owner.offsets.set(path, offset);
    clearTimeout(settle.current);
    settle.current = setTimeout(() => setScroll({ identity: owner.identity,
      lists: [...owner.offsets].map(([path, offset]) => ({ path, offset })) }), SCROLL_SETTLE_MS);
  }, []);
  const offsetOf = useCallback((path: string, drawn: number) => pending.current.offsets.get(path) ?? drawn, []);

  const fit = useRef<HTMLDivElement>(null);
  const available = useAvailable(fit);
  const frame = (body: ReactNode, size?: readonly [number, number]) => <div ref={fit} className="game-window"
    style={size ? { '--game-window-width': `${size[0]}px`, '--game-window-height': `${size[1]}px` } as CSSProperties : undefined}>{body}</div>;

  if (!reply) return frame(failed ? <WidgetState kind="error" what="this window" detail={failed} onRetry={retry} />
    : <WidgetState kind="loading" label="Loading window…" />);
  if (!scene) return frame(<WidgetState kind="unavailable" message={reply.refused.length ? reply.refused.join(' ') : undefined} />);
  return frame(<SceneView scene={scene} title={title} images={artwork.images} tints={artwork.tints} version={artwork.version}
    zoom={fitZoom(scene.size, available)} offsetOf={offsetOf} onScroll={onScroll} />, scene.size);
}

/** The CSS pixels the element has, followed as it resizes. */
function useAvailable(ref: RefObject<HTMLDivElement | null>): [number, number] {
  const [size, setSize] = useState<[number, number]>([0, 0]);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => setSize(previous => previous[0] === element.clientWidth && previous[1] === element.clientHeight
      ? previous : [element.clientWidth, element.clientHeight]);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
  return size;
}

/** The scene's texture pages: those not yet held, a few per request, every batch at once, kept
 * in the shared page cache under `owner` while this window draws from them; then the pages
 * its quads draw multiplied by a colour. */
function useArtwork(client: DashboardClient, sourceId: string, revision: string, owner: string | null, session: string | null, scene: Scene | null,
  active: boolean) {
  // A shown window follows its own owner's pages, whichever window loaded or coloured them.
  const follow = useCallback((listener: () => void) => active && owner ? pageCache.subscribe(owner, listener) : () => {}, [active, owner]);
  const pagesRevision = useCallback(() => owner ? pageCache.revision(owner) : 0, [owner]);
  const version = useSyncExternalStore(follow, pagesRevision, pagesRevision);
  const [warnings, setWarnings] = useState<{ owner: string | null; lines: string[] }>({ owner: null, lines: NO_LINES });
  const asked = useRef<{ owner: string | null; ids: Set<number>; tints: Set<string>; controller: AbortController } | null>(null);
  useEffect(() => {
    if (!active || !owner) return;
    pageCache.acquire(owner);
    return () => { asked.current?.controller.abort(); pageCache.release(owner); };
  }, [client, sourceId, revision, owner, active]);
  useEffect(() => {
    if (!active || !owner || !session || !scene) return;
    if (asked.current?.owner !== owner || asked.current.controller.signal.aborted)
      asked.current = { owner, ids: new Set(), tints: new Set(), controller: new AbortController() };
    const { ids, tints, controller } = asked.current;
    const note = (lines: string[]) => setWarnings(previous => ({ owner, lines: [...previous.owner === owner ? previous.lines : [], ...lines] }));
    const held = pageCache.pages(owner);
    for (const batch of batches(missingTextures(scene, id => held.has(id) || ids.has(id)))) {
      for (const id of batch) ids.add(id);
      const request: ProductRequest<GameWindowArtworkParams> = { sourceId, revision, product: 'game-window-artwork', params: { session, textures: batch } };
      client.product<GameWindowArtworkReply>(request, controller.signal).then(reply => {
        if (controller.signal.aborted) return;
        const data = validateProductReply(reply, request).data;
        for (const item of data.items) pageCache.add(owner, item);
        const lines = [...data.warnings, ...data.items.flatMap(item => item.unavailable ? [`Texture page ${item.id}: ${item.unavailable}`] : [])];
        if (lines.length) note(lines);
      }).catch((error: unknown) => {
        if (controller.signal.aborted) return;
        // Asked again by the next scene that draws them.
        for (const id of batch) ids.delete(id);
        note([`Texture pages ${batch.join(', ')} couldn’t load: ${message(error)}`]);
      });
    }
    // Pages drawn times a colour, once their page has arrived.
    const made = pageCache.tinted(owner);
    for (const quad of scene.quads) {
      const name = tintName(quad), image = held.get(quad.texture)?.image;
      if (!tinted(quad) || !image || made.has(name) || tints.has(name)) continue;
      tints.add(name);
      tintImage(image, quad.color).then(({ url, bytes }) => {
        if (controller.signal.aborted) { URL.revokeObjectURL(url); return; }
        // Kept, or another window's copy kept first: either way the cache tells every window.
        pageCache.addTinted(owner, name, url, bytes);
      }).catch((error: unknown) => {
        if (controller.signal.aborted) return;
        tints.delete(name);
        note([`Texture page ${quad.texture} couldn’t be coloured: ${message(error)}`]);
      });
    }
  }, [client, sourceId, revision, owner, session, scene, active, version]);
  return {
    images: owner ? pageCache.pages(owner) : EMPTY_PAGES, tints: owner ? pageCache.tinted(owner) : EMPTY_TINTS, version,
    warnings: warnings.owner === owner ? warnings.lines : NO_LINES,
  };
}

/** A page multiplied by a colour as the game's draw multiplies it ({@link tintPixels}). */
async function tintImage(url: string, color: readonly number[]): Promise<{ url: string; bytes: number }> {
  const image = new Image();
  image.src = url;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = image.naturalWidth;
  canvas.height = image.naturalHeight;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (!context) throw new Error('no 2D canvas');
  context.drawImage(image, 0, 0);
  const pixels = context.getImageData(0, 0, canvas.width, canvas.height, { colorSpace: 'srgb' });
  tintPixels(pixels.data, color);
  context.putImageData(pixels, 0, 0);
  const blob = await new Promise<Blob>((done, fail) => canvas.toBlob(blob => blob ? done(blob) : fail(new Error('the page could not be encoded')), 'image/png'));
  return { url: URL.createObjectURL(blob), bytes: blob.size };
}

const EMPTY_PAGES: ReadonlyMap<number, WindowTextureImage> = new Map();
const EMPTY_TINTS: ReadonlyMap<string, string> = new Map();

type Clip = readonly [number, number, number, number];

/** Children drawn inside a clip rectangle, still placed in window pixels. */
function Clipped({ clip, children }: { clip: Clip | undefined; children: ReactNode }) {
  if (!clip) return children;
  return <div className="game-window-clip" style={{ left: clip[0], top: clip[1], width: Math.max(0, clip[2] - clip[0]), height: Math.max(0, clip[3] - clip[1]) }}>
    <div className="game-window-origin" style={{ left: -clip[0], top: -clip[1] }}>{children}</div>
  </div>;
}

/** Consecutive hover areas sharing one clip, in draw order. */
function hoverRuns(hovers: readonly SceneHover[]) {
  const runs: { clip: number | undefined; start: number; hovers: SceneHover[] }[] = [];
  hovers.forEach((hover, index) => {
    const last = runs[runs.length - 1];
    if (last && last.clip === hover.clip) last.hovers.push(hover);
    else runs.push({ clip: hover.clip, start: index, hovers: [hover] });
  });
  return runs;
}

const QuadRun = memo(function QuadRun({ run, clip, textures, images, tints }: {
  run: ClipRun; clip: Clip | undefined; textures: ReadonlyMap<number, SceneTexture>; images: ReadonlyMap<number, WindowTextureImage>;
  tints: ReadonlyMap<string, string>;
  /** Changes as pages arrive in `images`. */
  version: number;
}) {
  const quads = run.quads.map((quad, index) => {
    // A page not loaded (or not yet coloured) leaves its quads undrawn.
    const texture = textures.get(quad.texture);
    const image = tinted(quad) ? tints.get(tintName(quad)) : images.get(quad.texture)?.image;
    const style = texture && image ? quadStyle(quad, texture, image) : null;
    return style && <div key={run.start + index} className="game-window-quad" style={style} />;
  });
  return <Clipped clip={clip}>{run.muted ? <div className="game-window-muted">{quads}</div> : quads}</Clipped>;
});

/** A scene drawn from the pages loaded so far, with its hover areas on top. */
export const SceneView = memo(function SceneView({ scene, title, images, tints, version, zoom, offsetOf, onScroll }: {
  scene: Scene; title: string | null; images: ReadonlyMap<number, WindowTextureImage>; tints: ReadonlyMap<string, string>; version: number; zoom: number;
  offsetOf: (path: string, drawn: number) => number; onScroll: (path: string, offset: number) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const runs = useMemo(() => clipRuns(scene.quads), [scene]);
  const textures = useMemo(() => new Map(scene.textures.map(texture => [texture.id, texture])), [scene]);
  const hovers = useMemo(() => hoverRuns(scene.hovers), [scene]);
  const clipOf = (index: number | undefined) => index === undefined ? undefined : scene.clips[index];

  // The wheel over a list moves it; the page around the window does not scroll meanwhile.
  const latest = useRef({ scene, offsetOf, onScroll });
  latest.current = { scene, offsetOf, onScroll };
  useEffect(() => {
    const element = box.current;
    if (!element) return;
    const wheel = (event: WheelEvent) => {
      const { scene, offsetOf, onScroll } = latest.current;
      const bounds = element.getBoundingClientRect();
      // Drawn size over interface size: the zoom as the pointer sees it.
      const scale = bounds.width / scene.size[0];
      if (!(scale > 0)) return;
      const list = listAt(scene, (event.clientX - bounds.left) / scale, (event.clientY - bounds.top) / scale);
      if (!list || list.max <= 0) return;
      event.preventDefault();
      const offset = offsetOf(list.path, list.offset);
      // CSS pixels over the window's zoom are window pixels, over the list's scale offset units.
      const next = scrolledOffset(offset, wheelPixels(event.deltaY, event.deltaMode, list.rect[list.horizontal ? 2 : 3] * scale), scale * (list.scale > 0 ? list.scale : 1),
        list.max);
      if (next !== offset) onScroll(list.path, next);
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, []);

  // A row focused outside its list's box scrolls its own list to it, along that list's axis.
  const reveal = (hover: SceneHover) => {
    const list = scene.lists.find(list => list.path === hover.list);
    if (!list || list.max <= 0) return;
    const next = revealedOffset(list.offset, list, hover.rect);
    if (next !== offsetOf(list.path, list.offset)) onScroll(list.path, next);
  };

  // Keys scroll the list a focused row sits in, or the first that scrolls when the window itself
  // is focused: a line by the row's height, a page by the list's, or to either end.
  const scrollable = scene.lists.some(list => list.max > 0);
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement;
    const path = target.dataset.list;
    const list = path === undefined ? scene.lists.find(list => list.max > 0) : scene.lists.find(list => list.path === path);
    if (!list || list.max <= 0) return;
    const offset = offsetOf(list.path, list.offset);
    const next = keyedOffset(event.key, offset, list, Number(target.dataset.line) || WHEEL_LINE_PIXELS);
    if (next === null) return;
    event.preventDefault();
    if (next !== offset) onScroll(list.path, next);
  };

  return <div ref={box} className="game-window-scene" role="group" aria-label={title ?? 'Game window'} tabIndex={scrollable ? 0 : undefined}
    onKeyDown={onKeyDown} style={{ width: scene.size[0], height: scene.size[1], zoom }}>
    <div className="game-window-layer" aria-hidden="true">
      {runs.map(run => <QuadRun key={run.start} run={run} clip={clipOf(run.clip)} textures={textures} images={images} tints={tints} version={version} />)}
    </div>
    <div className="game-window-layer is-hovers" role="list">
      {hovers.map(run => <Clipped key={run.start} clip={clipOf(run.clip)}>
        {run.hovers.map((hover, index) => <span key={run.start + index} className="game-window-hover" role="listitem" tabIndex={0} onFocus={() => reveal(hover)}
          aria-label={described(hover)} data-list={hover.list}
          data-line={Math.round(hover.rect[scene.lists.find(list => list.path === hover.list)?.horizontal ? 2 : 3])}
          style={{ left: hover.rect[0], top: hover.rect[1], width: hover.rect[2], height: hover.rect[3] }}
          data-tooltip={[hover.title, ...hover.details].join('\n')} />)}
      </Clipped>)}
    </div>
  </div>;
});
