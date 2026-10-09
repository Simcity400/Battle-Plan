import type { DashboardClient, ProductRequest } from './client';
import { loadTransientProduct } from './products.ts';
import { recordValue, textValue, i32Value } from './payload.ts';
import type { OperationDescriptionRef, OperationDescriptionReply } from '../generated/dashboard';

const unsigned = (value: unknown): value is string => typeof value === 'string' && /^(0|[1-9]\d*)$/.test(value) && BigInt(value) <= 18446744073709551615n;
const same = (left: OperationDescriptionRef, right: unknown): boolean => recordValue(right)
  && right.hour === left.hour && right.id === left.id && right.owner === left.owner && right.sequence === left.sequence;

export function validateDescriptionPage(data: unknown, reference: OperationDescriptionRef, offset: string): OperationDescriptionReply {
  if (!recordValue(data) || !same(reference, data.description) || data.offset !== offset
    || !textValue(data.recordedLanguage) || !unsigned(data.totalBytes)
    || !Array.isArray(data.bytes) || !data.bytes.every(byte => i32Value(byte) && byte >= 0 && byte <= 255)
    || !(data.unavailable === null || textValue(data.unavailable))
    || !(data.nextOffset === null || unsigned(data.nextOffset))) throw new Error('The operation description page is malformed.');
  const first = BigInt(offset), last = first + BigInt(data.bytes.length), total = BigInt(data.totalBytes);
  if (first > total || last > total || (data.unavailable !== null && (data.bytes.length || data.nextOffset !== null))
    || (data.unavailable === null && (data.nextOffset === null ? last !== total : data.bytes.length === 0 || BigInt(data.nextOffset) !== last || last >= total))) {
    throw new Error('The operation description page does not preserve its exact byte range.');
  }
  return data as unknown as OperationDescriptionReply;
}

/** Reads just the requested description; pages never occupy the branch-product cache. */
export async function readDescriptionPages(reference: OperationDescriptionRef, page: (offset: string) => Promise<unknown>, signal: AbortSignal): Promise<{ text: string; recordedLanguage: string }> {
  const decoder = new TextDecoder('utf-8', { fatal: true });
  const pieces: string[] = [];
  let offset = '0', total: string | undefined, language: string | undefined;
  for (;;) {
    signal.throwIfAborted();
    const next = validateDescriptionPage(await page(offset), reference, offset);
    if (total !== undefined && (next.totalBytes !== total || next.recordedLanguage !== language)) {
      throw new Error('The recorded operation description changed between pages.');
    }
    total = next.totalBytes; language = next.recordedLanguage;
    if (next.unavailable !== null) throw new Error(next.unavailable);
    try { pieces.push(decoder.decode(Uint8Array.from(next.bytes), { stream: next.nextOffset !== null })); }
    catch { throw new Error('Stored operation description refused: non-UTF-8 recorded bytes'); }
    if (next.nextOffset === null) return { text: pieces.join(''), recordedLanguage: language };
    offset = next.nextOffset;
  }
}
export function loadOperationDescription(client: DashboardClient, source: Pick<ProductRequest, 'sourceId' | 'revision'>, branchId: string,
  reference: OperationDescriptionRef, signal: AbortSignal) {
  return readDescriptionPages(reference, offset => loadTransientProduct(client, { ...source, product: 'operation-description',
    params: { branchId, description: reference, offset } }, signal), signal);
}
