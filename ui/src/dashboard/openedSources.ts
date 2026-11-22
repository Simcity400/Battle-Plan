import type { DashboardSource, ProductRequest } from './client';
import { hourFields, refusedHours } from './regionTime.ts';

const ENVELOPES = new Set(['follow-clock', 'unfollow']);

export class OpenedSources {
  private readonly sources = new Map<string, DashboardSource>();

  add(source: DashboardSource): DashboardSource { this.sources.set(source.id, source); return source; }
  delete(sourceId: string): void { this.sources.delete(sourceId); }

  refusal(request: Pick<ProductRequest, 'sourceId' | 'product' | 'params'>): string | null {
    if (ENVELOPES.has(request.product)) return null;
    const params = request.params as Readonly<Record<string, unknown>> | null | undefined;
    const followed = request.product === 'follow';
    const asked = followed ? params?.query : params;
    let dated = followed;
    hourFields(asked, () => { dated = true; });
    if (!dated) return null;
    const branchId = params?.branchId;
    const branch = this.sources.get(request.sourceId)?.metadata.branches.find(item => item.id === branchId);
    if (!branch) return `${request.product} names hours on a branch this window has not opened`;
    const refused = refusedHours(asked, branch, followed);
    return refused && `${request.product}: ${refused}`;
  }
}
