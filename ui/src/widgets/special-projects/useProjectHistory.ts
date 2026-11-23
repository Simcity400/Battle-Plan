import type { GatedParams } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import type { Gate } from '../../dashboard/regionGate';
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { RefObject } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { followStore } from '../../dashboard/follow/store.ts';
import { panelReply } from '../../dashboard/follow/panel.ts';
import { useProducts } from '../../dashboard/products.ts';
import type { Failure } from '../../dashboard/queryError';
import { useFollowedAll } from '../scoreboard/followedAll.ts';
import type { SpecialProjectsReply } from '../../generated/dashboard';
import { HISTORY_PANEL, heldThrough, validateSpecialProjectsReply } from './specialProjectsData.ts';

export interface HistoryReading { replies: readonly (SpecialProjectsReply | undefined)[]; hour: number | null; through: number | null; error?: Failure; refused?: string; outside?: boolean }

export interface ProjectHistoryFeed {
  replies: SpecialProjectsReply[] | undefined;
  hour: number | null;
  error?: Failure;
  refused?: string;
  outside?: boolean;
  retry: () => void;
  latest: () => HistoryReading;
  subscribe: (changed: () => void) => () => void;
}

type Params = Readonly<Record<string, unknown>>;
type Picture = (replies: readonly SpecialProjectsReply[], hour: number) => string;

const ready = (reading: HistoryReading): reading is HistoryReading & { replies: SpecialProjectsReply[]; hour: number } =>
  !reading.error && reading.refused === undefined && !reading.outside && reading.hour !== null && reading.replies.length > 0 && reading.replies.every(reply => reply !== undefined);

export function readingKey(reading: HistoryReading, picture: Picture): string {
  if (reading.outside) return 'outside';
  if (reading.refused !== undefined) return `?${reading.refused}`;
  if (reading.error) return `!${reading.error.message}`;
  return ready(reading) ? `=${picture(reading.replies, reading.hour)}` : '';
}

export function useProjectHistory(runtime: WidgetRuntime, gate: Pick<Gate, 'follow' | 'recorded'>, track: BoundTrack | undefined, params: readonly Params[], following: boolean, active: boolean,
  at: number, picture: Picture, root: RefObject<Element | null>): ProjectHistoryFeed {
  const followed = following && active && !!track;
  const store = useMemo(() => followed && track ? followStore(runtime.client, track, runtime.clock) : null,
    [followed, runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const queries = useMemo(() => followed ? params.map(param => gate.follow({ ...param, panel: HISTORY_PANEL })) : [], [followed, params, gate.follow]);
  const all = useFollowedAll(store, queries, root);
  const read = useCallback((): HistoryReading => {
    let error = all.error;
    let refused = all.reason;
    const replies = all.entries.map(entry => {
      if (!entry?.panel) return undefined;
      if (entry.panel.refused !== null) { refused ??= entry.panel.refused; return undefined; }
      const shown = panelReply(entry.panel, validateSpecialProjectsReply);
      error ??= shown.error;
      return shown.data && heldThrough(shown.data, entry.hour, entry.through);
    });
    const held = all.entries.find(entry => entry?.panel);
    return { replies, hour: held?.hour ?? null, through: held?.through ?? null, ...error ? { error } : {}, ...refused !== undefined ? { refused } : {}, ...all.outside ? { outside: true } : {} };
  }, [all]);
  const [shown, setShown] = useState<{ key: string; reading: HistoryReading }>({ key: '', reading: { replies: [], hour: null, through: null } });
  useLayoutEffect(() => {
    if (!followed) return;
    const update = () => {
      const reading = read(), key = readingKey(reading, picture);
      setShown(held => held.key === key ? held : { key, reading });
    };
    update();
    return all.subscribe(update);
  }, [followed, read, picture, all.subscribe]);
  const askedAt = gate.recorded(at);
  const requests = useMemo(() => !followed && track && askedAt !== null ? params.map(param => productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'special-projects', params: { ...param, hour: askedAt } as GatedParams })) : [], [followed, track, params, askedAt]);
  const asked = useProducts<SpecialProjectsReply>(runtime.client, requests, active && !followed, validateSpecialProjectsReply, track?.source);
  const askedLatest = useRef<HistoryReading>({ replies: [], hour: null, through: null });
  askedLatest.current = { replies: asked.data ?? [], hour: asked.data ? at : null, through: asked.data ? at : null };
  const askedRead = useCallback(() => askedLatest.current, []);
  const quiet = useCallback(() => () => {}, []);
  if (followed) {
    const reading = shown.reading;
    return { replies: ready(reading) ? reading.replies : undefined, hour: reading.hour, ...reading.error ? { error: reading.error } : {},
      ...reading.refused !== undefined ? { refused: reading.refused } : {}, ...reading.outside ? { outside: true } : {}, retry: all.retry, latest: read, subscribe: all.subscribe };
  }
  return { replies: asked.data, hour: asked.data ? at : null, ...asked.error ? { error: asked.error } : {}, retry: asked.retry, latest: askedRead, subscribe: quiet };
}
