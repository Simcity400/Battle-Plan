import type { DashboardSource, ProductRequest } from './client';
import { followedProduct, hourFields, refusedHours } from './regionTime.ts';

const ENVELOPES = new Set(['follow-clock', 'unfollow', 'follow-close']);

export class OpenedSources {
  private readonly sources = new Map<string, DashboardSource>();

  add(source: DashboardSource): DashboardSource { this.sources.set(source.id, source); return source; }
  delete(sourceId: string): void { this.sources.delete(sourceId); }

  refusal(request: Pick<ProductRequest, 'sourceId' | 'product' | 'params'>): string | null {
    if (ENVELOPES.has(request.product)) return null;
    const params = request.params as Readonly<Record<string, unknown>> | null | undefined;
    const followed = request.product === 'follow';
    const asked = followed ? followedProduct(params?.query) : { product: request.product, params };
    if (!asked) return `${request.product} names no followed product`;
    let dated = false;
    if (!hourFields(asked.product, asked.params, () => { dated = true; })) return `${request.product}: ${asked.product} is not a declared product`;
    if (!dated) return null;
    const branchId = params?.branchId;
    const branch = this.sources.get(request.sourceId)?.metadata.branches.find(item => item.id === branchId);
    if (!branch) return `${request.product} names hours on a branch this window has not opened`;
    const refused = refusedHours(asked.product, asked.params, branch, followed);
    return refused && `${request.product}: ${refused}`;
  }
}
