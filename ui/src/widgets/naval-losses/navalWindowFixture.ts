/** SSR presentation tests preload reached transport data. The mounted Pin test
 * executes the real prepared hook and shared window effects. */
export const navalWindowFixture = `import { useMemo } from 'react';
  import { useProduct } from './products';
  const empty = new Map();
  export function useProductWindows(client, identity, request) {
    const reply = useProduct(client, identity ? request(0) : null);
    const data = useMemo(() => reply.data ? new Map([[0,reply.data]]) : empty,[reply.data]);
    return { ...reply, data, errors: empty };
  }
  export const useWindowDisplay = (identity,data,hour) => identity && data ? {identity,data,hour} : null;`;
