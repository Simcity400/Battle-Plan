import { memo, useCallback, useEffect, useId, useLayoutEffect, useMemo, useReducer, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent, ReactNode, RefObject } from 'react';
import { validateProductReply } from '../dashboard/client';
import type { DashboardClient, ProductRequest } from '../dashboard/client';
import { useContentRevision } from '../dashboard/contentRevision';
import WidgetState from './WidgetState';
import { WHEEL_LINE_PIXELS, described, fitZoom, keyedOffset, listAt, revealedOffset, scrolledOffset, wheelPixels } from './gameWindowScene';
import type { GameWindowParams, GameWindowReply, GameWindowTarget, ListScroll, Scene, SceneHover } from '../generated/dashboard';
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

/** One game window, laid out and painted by the service as the game draws it at the hour shown.
 * Its image is fitted whole into the space it has by zoom, never laid out again here: scrolling
 * a list asks the service for the window at the new offset. That the window is read-only is
 * described to assistive technology only; nothing is drawn over or beside the game's image. */
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
    client.product<GameWindowReply>(request, controller.signal).then(async reply => {
      if (controller.signal.aborted) return;
      const data = validateProductReply(reply, request).data;
      // The new drawing is decoded before it replaces the old, so hover areas never sit over
      // the previous window's image.
      if (data.scene) await decoded(data.scene.image);
      if (controller.signal.aborted) return;
      setShown({ identity, key, reply: data });
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

  const title = reply?.title ?? null;
  useEffect(() => { onTitle?.(title); }, [onTitle, title]);
  const failed = failure?.identity === identity ? failure.detail : null;
  useEffect(() => {
    if (!onDiagnostics) return;
    // Hidden, it reports nothing; what it reported belonged to what was shown.
    const lines = !active ? NO_LINES : [...reply?.warnings ?? [], ...reply?.refused ?? [], ...scene?.notes ?? [],
      ...(reply?.slots ?? []).filter(slot => slot.state !== 'bound').map(slot => `${slot.name}: ${slot.detail ?? (slot.state === 'refused' ? 'refused' : 'not filled')}`),
      ...reply && failed ? [`Couldn’t load the window again: ${failed}`] : []];
    onDiagnostics(lines.length ? lines : NO_LINES);
  }, [onDiagnostics, active, reply, scene, failed]);
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
  return frame(<SceneView scene={scene} title={title} zoom={fitZoom(scene.size, available)} offsetOf={offsetOf} onScroll={onScroll} />, scene.size);
}

/** Resolves once `url` is decoded and ready to draw. */
async function decoded(url: string): Promise<void> {
  const image = new Image();
  image.src = url;
  await image.decode();
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

type Clip = readonly [number, number, number, number];

/** Children drawn inside a clip rectangle, still placed in window pixels. */
function Clipped({ clip, children }: { clip: Clip | undefined; children: ReactNode }) {
  if (!clip) return children;
  return <div className="game-window-clip" style={{ left: clip[0], top: clip[1], width: Math.max(0, clip[2] - clip[0]), height: Math.max(0, clip[3] - clip[1]) }}>
    <div className="game-window-origin" style={{ left: -clip[0], top: -clip[1] }}>{children}</div>
  </div>;
}

const sameClip = (a: Clip | undefined, b: Clip | undefined) => a === b || (!!a && !!b && a.every((edge, index) => edge === b[index]));

/** Consecutive hover areas sharing one clip, in draw order. */
function hoverRuns(hovers: readonly SceneHover[]) {
  const runs: { clip: Clip | undefined; start: number; hovers: SceneHover[] }[] = [];
  hovers.forEach((hover, index) => {
    const last = runs[runs.length - 1];
    if (last && sameClip(last.clip, hover.clip)) last.hovers.push(hover);
    else runs.push({ clip: hover.clip, start: index, hovers: [hover] });
  });
  return runs;
}

/** A window's image as the game draws it, with its hover areas on top; that it is read-only is
 * a description for assistive technology, never drawn. */
export const SceneView = memo(function SceneView({ scene, title, zoom, offsetOf, onScroll }: {
  scene: Scene; title: string | null; zoom: number;
  offsetOf: (path: string, drawn: number) => number; onScroll: (path: string, offset: number) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const hovers = useMemo(() => hoverRuns(scene.hovers), [scene]);
  const description = useId();

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

  return <div className="game-window-view">
    <div ref={box} className="game-window-scene" role="group" aria-label={title ?? 'Game window'} aria-describedby={description}
      tabIndex={scrollable ? 0 : undefined} onKeyDown={onKeyDown} style={{ width: scene.size[0], height: scene.size[1], zoom }}>
      <img className="game-window-image" src={scene.image} alt="" draggable={false} />
      <div className="game-window-layer is-hovers" role="list">
      {hovers.map(run => <Clipped key={run.start} clip={run.clip}>
        {run.hovers.map((hover, index) => <span key={run.start + index} className="game-window-hover" role="listitem" tabIndex={0} onFocus={() => reveal(hover)}
          aria-label={described(hover)} data-list={hover.list}
          data-line={Math.round(hover.rect[scene.lists.find(list => list.path === hover.list)?.horizontal ? 2 : 3])}
          style={{ left: hover.rect[0], top: hover.rect[1], width: hover.rect[2], height: hover.rect[3] }}
          data-tooltip={[hover.title, ...hover.details].join('\n')} />)}
      </Clipped>)}
      </div>
    </div>
    <span id={description} className="sr-only">{READ_ONLY}</span>
  </div>;
});

/** What assistive technology is told of a window: nothing in it acts. */
const READ_ONLY = 'Read-only: the game’s window as it stood at this hour. Nothing in it acts here.';
