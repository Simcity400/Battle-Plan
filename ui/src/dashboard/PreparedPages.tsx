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

export interface PreparedPage { id: string; template: DashboardTemplate; runtime: WidgetRuntime }

const SETTLED_FRAMES = 2;

interface Props {
  pages: readonly PreparedPage[];
  clock: PlaybackController;
  client: DashboardClient;
  viewport: HTMLElement | null;
  enabled: boolean;
  draw: (page: PreparedPage) => ReactNode;
  mounting?: () => number;
}

const none = () => 0;

export default function PreparedPages({ pages, clock, client, viewport, enabled, draw, mounting = none }: Props) {
  const available = enabled && pages.length > 0 && viewport !== null;
  const paused = useSyncExternalStore(clock.subscribe, () => {
    const snapshot = clock.getSnapshot();
    return available && !snapshot.playing ? snapshot : null;
  });
  const on = paused !== null;
  const [round, setRound] = useState<{ index: number; width: number; height: number; token: object } | null>(null);
  const gate = prepareGate(client);
  const owners = useRef(new Map<string, object>());
  const owner = (id: string) => {
    let held = owners.current.get(id);
    if (!held) owners.current.set(id, held = {});
    return held;
  };

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
  }, [on, pages, paused?.hour, paused?.seekSequence, viewport]);

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
    return () => cancelAnimationFrame(frame);
  }, [page, round?.token, index]);

  useEffect(() => {
    const ids = new Set(pages.map(entry => entry.id));
    for (const [id, held] of [...owners.current]) if (!ids.has(id)) { unpinProducts(client, held); owners.current.delete(id); }
  }, [pages, client]);
  useEffect(() => () => { for (const held of owners.current.values()) unpinProducts(client, held); owners.current.clear(); }, [client]);

  if (!on || !round || !page) return null;
  return <div className="dash-prepare" aria-hidden="true" data-prepare={page.id} style={{ width: round.width, height: round.height }}>
    <div className="dash-prepare-scroll"><PreparingPage.Provider value={true}><LiveMotion.Provider value={clock.getMotion}>
      <div key={`${page.id}`} style={{ display: 'contents' }}>{draw(page)}</div>
    </LiveMotion.Provider></PreparingPage.Provider></div>
  </div>;
}
