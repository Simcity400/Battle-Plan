import { artworkUrlValue } from '../../dashboard/payload.ts';
import type { ResourceBalanceReply, ResourceInfo } from '../../generated/dashboard';

export const RESOURCE_BALANCE = 'resource-balance';

const RAW = /^-?\d{1,40}$/;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const optionalRaw = (value: unknown) => value === null || (typeof value === 'string' && RAW.test(value));

export function validateResourceBalance(value: unknown): ResourceBalanceReply {
  const reply = value as ResourceBalanceReply;
  const ids = object(value) && Array.isArray(value.resources) ? new Set(value.resources.map(item => object(item) ? item.id : null)) : new Set();
  const known = (id: unknown) => ids.has(id);
  const ok = object(value) && typeof reply.tag === 'string' && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.resources) && reply.resources.every((item, index) => object(item) && Number.isInteger(item.id) && typeof item.token === 'string'
      && item.token.length > 0 && Number.isInteger(item.ordinal) && item.ordinal >= 1 && (item.name === null || typeof item.name === 'string')
      && (item.icon === null || artworkUrlValue(item.icon)) && (index === 0 || reply.resources[index - 1]!.id < item.id))
    && Array.isArray(reply.pools) && reply.pools.every(known)
    && Array.isArray(reply.rows) && reply.rows.every(row => object(row) && known(row.resource) && optionalRaw(row.net) && optionalRaw(row.supply) && optionalRaw(row.demand)
      && (row.missing === null || typeof row.missing === 'string') && (row.missing === null) === (row.net !== null && row.supply !== null && row.demand !== null))
    && new Set(reply.rows.map(row => row.resource)).size === reply.rows.length
    && Number.isInteger(reply.short) && reply.short >= 0 && Number.isInteger(reply.unrecorded) && reply.unrecorded >= 0
    && (reply.poolsMissing === null || typeof reply.poolsMissing === 'string') && (reply.dealsMissing === null || typeof reply.dealsMissing === 'string')
    && Array.isArray(reply.deals) && reply.deals.every(deal => object(deal) && typeof deal.deal === 'string' && typeof deal.partner === 'string'
      && known(deal.resource) && (deal.direction === 'import' || deal.direction === 'export')
      && Number.isInteger(deal.factories) && Number.isInteger(deal.convoysAssigned) && Number.isInteger(deal.convoysRequired))
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The resource balance reply is malformed.');
  return reply;
}

export const resourceName = (item: Pick<ResourceInfo, 'name' | 'token' | 'ordinal'>) => `${item.name ?? 'Resource name unavailable'}${item.ordinal > 1 ? ` (${item.ordinal})` : ''}`;
