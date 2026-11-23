import type { RequestParams } from '../generated/dashboard';
import type { BoundHour, GatedHour, ParamsFor } from './regionTime';

declare const MADE: unique symbol;

export interface AskedRequest<K extends string = string> {
  sourceId: string;
  revision: string;
  product: K;
  params: ParamsFor<K>;
  asksFrom?: number;
}

export type ProductRequest<K extends string = string> = AskedRequest<K> & { readonly [MADE]: 'made' };

export type DeclaredProduct = keyof RequestParams<GatedHour, BoundHour> & string;

type Named<K extends string> = string extends K ? { product: never } : unknown;

export const productRequest = <const K extends string>(request: AskedRequest<K> & Named<K>): ProductRequest<NoInfer<K>> => request as unknown as ProductRequest<NoInfer<K>>;

export const forwardRequest = <K extends DeclaredProduct>(source: { readonly id: string; readonly revision: string }, product: K, params: ParamsFor<K>,
  asksFrom?: number): ProductRequest<K> => ({ sourceId: source.id, revision: source.revision, product, params, ...(asksFrom === undefined ? {} : { asksFrom }) }) as ProductRequest<K>;
