import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';
import type { DashboardClient } from './client';
import type { WidgetRuntime } from './definitions';
import type { DashboardTemplate } from './model';
import type { PlaybackController } from './playback';
import { prepareGate } from './prepare.ts';
import { pinProducts, unpinProducts } from './products.ts';
import { PreparingPage } from './preparing.ts';
import { LiveMotion } from '../charts/liveMotion.ts';
import { busyCount } from '../shell/busy.ts';

/** A page the lane prepares: its layout and the runtime its widgets read through the lane's client. */
export interface PreparedPage { id: string; template: DashboardTemplate; runtime: WidgetRuntime }

/** Frames in a row with nothing in flight before the lane starts, and before a page counts as prepared: one for the
 * last reply's commit, one for the effects it starts. */
const SETTLED_FRAMES = 2;

interface Props {
  /** In order: the page shown (its rows below the fold), then the others from the next. */
  pages: readonly PreparedPage[];
  clock: PlaybackController;
  client: DashboardClient;
  /** The page's scrolled view: a prepared copy is laid out at its size, so its widgets ask what they would there. */
  viewport: HTMLElement | null;
  enabled: boolean;
  /** Draws a page unseen: the dashboard's grid, prepared. */
  draw: (page: PreparedPage) => ReactNode;
  /** Widget bodies still waiting their turn to mount. */
  mounting?: () => number;
}

const none = () => 0;

/** The prepare lane: while the clock is paused and the page has settled, each page in turn is laid out unseen and its
 * widgets ask through the lane what they show, until nothing of it is in flight; its replies stay held for that page until
 * it is prepared again. Playing, or a seek, unmounts the copy, which cancels what it asked. */
export default function PreparedPages({ pages, clock, client, viewport, enabled, draw, mounting = none }: Props) {
  const snapshot = useSyncExternalStore(clock.subscribe, clock.getSnapshot);
  const on = enabled && !snapshot.playing && pages.length > 0 && viewport !== null;
  const [round, setRound] = useState<{ index: number; width: number; height: number; token: object } | null>(null);
  const gate = prepareGate(client);
  const owners = useRef(new Map<string, object>());
  const owner = (id: string) => {
    let held = owners.current.get(id);
    if (!held) owners.current.set(id, held = {});
    return held;
  };

  // A round starts once the page has settled at the hour held.
  useEffect(() => {
    setRound(null);
    if (!on) return;
    let frame = 0, settled = 0;
    const wait = () => {
      settled = busyCount() === 0 ? settled + 1 : 0;
      if (settled < SETTLED_FRAMES) { frame = requestAnimationFrame(wait); return; }
      const box = viewport!.getBoundingClientRect();
      setRound({ index: 0, width: box.width, height: box.height, token: {} });
    };
    frame = requestAnimationFrame(wait);
    return () => cancelAnimationFrame(frame);
  }, [on, pages, snapshot.hour, snapshot.seekSequence, viewport]);

  // The page laid out is prepared once every body has mounted and nothing is in flight, the lane's or the page's.
  const index = round?.index ?? -1;
  const page = index >= 0 ? pages[index] : undefined;
  useEffect(() => {
    if (!page) return;
    gate.drain();
    let frame = 0, settled = 0;
    const check = () => {
      settled = gate.pending === 0 && busyCount() === 0 && mounting() === 0 ? settled + 1 : 0;
      if (settled < SETTLED_FRAMES) { frame = requestAnimationFrame(check); return; }
      pinProducts(client, owner(page.id), gate.drain());
      setRound(current => current && current.index === index ? { ...current, index: index + 1 } : current);
    };
    frame = requestAnimationFrame(check);
    return () => {
      cancelAnimationFrame(frame);
      // The copy's widgets have left: what still waits in the lane is asked by the page itself.
      setTimeout(() => gate.release(), 0);
    };
    // A page is one turn of its round.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, round?.token, index]);

  // A page no longer in the campaign lets go of what it held.
  useEffect(() => {
    const ids = new Set(pages.map(entry => entry.id));
    for (const [id, held] of [...owners.current]) if (!ids.has(id)) { unpinProducts(client, held); owners.current.delete(id); }
  }, [pages, client]);
  useEffect(() => () => { for (const held of owners.current.values()) unpinProducts(client, held); owners.current.clear(); }, [client]);

  // Playing drops the copy in the commit that starts it: rendered once more, every widget of it would render as playing.
  if (!on || !round || !page) return null;
  return <div className="dash-prepare" aria-hidden="true" data-prepare={page.id} style={{ width: round.width, height: round.height }}>
    <div className="dash-prepare-scroll"><PreparingPage.Provider value={true}><LiveMotion.Provider value={clock.getMotion}>
      <div key={`${page.id}`} style={{ display: 'contents' }}>{draw(page)}</div>
    </LiveMotion.Provider></PreparingPage.Provider></div>
  </div>;
}
