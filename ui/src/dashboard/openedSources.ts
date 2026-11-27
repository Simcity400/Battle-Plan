import type { DashboardSource } from './client';
import { followedProduct, hourFields, refusedHours } from './regionTime.ts';

const ENVELOPES = new Set(['follow-clock', 'unfollow', 'follow-close', 'event-feed-title-view']);

function otherBranch(value: unknown, branchId: unknown): string | null {
  if (value === null || typeof value !== 'object') return null;
  if (Array.isArray(value)) { for (const item of value) { const other = otherBranch(item, branchId); if (other !== null) return other; } return null; }
  for (const [name, field] of Object.entries(value)) {
    if (name === 'branchId' && field !== branchId) return String(field);
    const other = otherBranch(field, branchId);
    if (other !== null) return other;
  }
  return null;
}

export class OpenedSources {
  private readonly sources = new Map<string, DashboardSource>();

  add(source: DashboardSource): DashboardSource { this.sources.set(source.id, source); return source; }
  delete(sourceId: string): void { this.sources.delete(sourceId); }

  refusal(request: { readonly sourceId: string; readonly product: string; readonly params: unknown }): string | null {
    if (ENVELOPES.has(request.product)) return null;
    const params = request.params as Readonly<Record<string, unknown>> | null | undefined;
    if (request.product === 'strip') {
      const { product, asks, whole } = params ?? {};
      if (typeof product !== 'string' || product === 'strip' || product === 'follow' || ENVELOPES.has(product) ||
          typeof whole !== 'boolean' || !Array.isArray(asks) || asks.length < 1 || asks.length > 17 ||
          asks.some(ask => !ask || typeof ask !== 'object' || Array.isArray(ask))) return 'Invalid strip request';
      for (const [i, params] of asks.entries()) {
        const refusal = this.refusal({ sourceId: request.sourceId, product, params });
        if (refusal) return `strip ask ${i + 1}: ${refusal}`;
      }
      return null;
    }
    const followed = request.product === 'follow';
    const asked = followed ? followedProduct(params?.query) : { product: request.product, params };
    if (!asked) return `${request.product} names no followed product`;
    let dated = false, shape: string | null = null;
    if (!hourFields(asked.product, asked.params, () => { dated = true; }, (path, expected) => { shape ??= `${path} is not ${expected}, as the contract declares`; })) {
      return `${request.product}: ${asked.product} is not a declared product`;
    }
    if (shape !== null) return `${request.product}: ${shape}`;
    const branchId = params?.branchId;
    const other = otherBranch(asked.params, branchId);
    if (other !== null) return `${request.product} names branch ${other}, not the ${followed ? 'followed ' : ''}branch ${String(branchId)}`;
    if (!dated) return null;
    const branch = this.sources.get(request.sourceId)?.metadata.branches.find(item => item.id === branchId);
    if (!branch) return `${request.product} names hours on a branch this window has not opened`;
    const refused = refusedHours(asked.product, asked.params, branch, followed);
    return refused && `${request.product}: ${refused}`;
  }
}
