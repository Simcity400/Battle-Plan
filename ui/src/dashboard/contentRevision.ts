import { useSyncExternalStore } from 'react';
import type { ContentFolders } from '../shell/api';
import type { DashboardClient } from './client';
import { PerClient } from './prepare.ts';

export class ContentRevision {
  private key: string | undefined;
  private presentation: number | undefined;
  private revision = 0;
  private listeners = new Set<() => void>();
  snapshot = (): number => this.revision;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };

  observe(folders: ContentFolders | null): void {
    if (!folders) return;
    const key = JSON.stringify([folders.game, folders.user, folders.contentReady]);
    if (key === this.key) return;
    this.key = key;
    this.advance();
  }

  observePresentation(generation: number): void {
    if (this.presentation !== undefined && generation <= this.presentation) return;
    const first = this.presentation === undefined;
    this.presentation = generation;
    if (!first) this.advance();
  }

  private advance(): void {
    this.revision++;
    for (const listener of this.listeners) listener();
  }
}

const revisions = new PerClient<ContentRevision>();
export function contentRevision(client: DashboardClient): ContentRevision {
  let revision = revisions.get(client);
  if (!revision) { revision = new ContentRevision(); revisions.set(client, revision); }
  return revision;
}

const noContentSubscription = () => () => {};
const zeroContentRevision = () => 0;

export function useContentRevision(client: DashboardClient, enabled = true): number {
  const revision = contentRevision(client);
  const subscribe = enabled ? revision.subscribe : noContentSubscription;
  const snapshot = enabled ? revision.snapshot : zeroContentRevision;
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
