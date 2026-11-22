import type { ParamsFor } from './regionTime';

declare const MADE: unique symbol;

export interface AskedRequest<K extends string = string> {
  sourceId: string;
  revision: string;
  product: K;
  params: ParamsFor<K>;
  asksFrom?: number;
}

export type ProductRequest<K extends string = string> = AskedRequest<K> & { readonly [MADE]: 'made' };

export const productRequest = <K extends string>(request: AskedRequest<K>): ProductRequest<K> => request as ProductRequest<K>;
