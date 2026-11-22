import { createContext } from 'react';
import type { DashboardClient, DashboardSource } from '../client';
import type { PlaybackController } from '../playback';
import { parseAnswer, type FollowedAnswer, type FollowedPanel } from './message.ts';
import type { Pipeline } from './pipeline';
import { installedAnswer, installedModel, installedPanel } from './useFollowed.ts';
import type { LiveMap } from '../../draw/views/fields';

export type FollowQuery = Readonly<Record<string, unknown>>;

export const FollowedWidgetKey = createContext<string | null>(null);

export interface FollowStore { readonly client: DashboardClient; readonly source: DashboardSource; readonly clock: PlaybackController; readonly offset: number }

export function followStore(client: DashboardClient, source: DashboardSource, clock: PlaybackController, offset = 0): FollowStore {
  return { client, source, clock, offset };
}

export interface FollowEntry {
  readonly id: number;
  readonly status: 'pending' | 'followed' | 'unavailable' | 'failed';
  readonly reason?: string;
  readonly answer: FollowedAnswer | undefined;
  readonly panel?: FollowedPanel;
  readonly values?: Map<string, number | bigint | string>;
  readonly structure?: string;
  readonly hour: number;
  readonly through: number;
  readonly version: number;
  subscribe(changed: () => void): () => void;
}

const answers = new WeakMap<Uint8Array, FollowedAnswer>();
const answerOf = (body: Uint8Array | undefined) => {
  if (!body) return undefined;
  let answer = answers.get(body);
  if (!answer) answers.set(body, answer = parseAnswer(body));
  return answer;
};

export function installedEntry(pipeline: Pipeline, widget: number, sub: number): FollowEntry {
  const view = () => pipeline.view(widget);
  const installed = () => { const shown = view(); return shown?.mode === 'picture' ? shown : null; };
  return {
    id: sub,
    get status() {
      const told = pipeline.status(sub);
      if (told.status === 'refused') return 'unavailable';
      const shown = view();
      return shown?.mode === 'failed' ? 'failed' : told.status === 'followed' ? 'followed' : 'pending';
    },
    get reason() { return pipeline.status(sub).reason ?? (view()?.mode === 'failed' ? view()?.reason : undefined); },
    get answer() { return answerOf(installedAnswer(installed(), sub)); },
    get panel() { return installedPanel(installedModel(installed(), sub)); },
    get values() { return (installed()?.fields?.[`live:${sub}`] as LiveMap | null | undefined) ?? undefined; },
    get structure() { return installedModel(installed(), sub)?.structure; },
    get hour() { return installed()?.h ?? 0; },
    get through() { return installed()?.through ?? 0; },
    get version() { return installed()?.id ?? 0; },
    subscribe: changed => pipeline.onInstall(widget, changed),
  };
}

export const shownThrough = (entry: FollowEntry): number | null => entry.answer === undefined && entry.panel === undefined ? null : entry.through;

export function queryBranch(query: FollowQuery): string | null {
  const branch = query.branchId ?? (query.params as { branchId?: unknown } | undefined)?.branchId;
  return typeof branch === 'string' ? branch : null;
}

export const watchEntry = (entry: FollowEntry, changed: () => void) => entry.subscribe(changed);
