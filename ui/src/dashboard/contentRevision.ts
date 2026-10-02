/** Content-dependent replies are scoped to the configured folders and readiness. */
import { useSyncExternalStore } from 'react';
import type { ContentFolders } from '../shell/api';
import type { DashboardClient } from './client';

export class ContentRevision {
  private key: string | undefined;
  private revision = 0;
  private listeners = new Set<() => void>();
  snapshot = (): number => this.revision;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };

  observe(folders: ContentFolders | null): void {
    if (!folders) return;
    const key = JSON.stringify([folders.game, folders.user, folders.contentReady]);
    if (key === this.key) return;
    this.key = key;
    this.revision++;
    for (const listener of this.listeners) listener();
  }
}

const revisions = new WeakMap<DashboardClient, ContentRevision>();
export function contentRevision(client: DashboardClient): ContentRevision {
  let revision = revisions.get(client);
  if (!revision) { revision = new ContentRevision(); revisions.set(client, revision); }
  return revision;
}

export function useContentRevision(client: DashboardClient): number {
  const revision = contentRevision(client);
  return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);
}
