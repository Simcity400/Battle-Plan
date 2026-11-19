import { useCallback, useState } from 'react';

export function useDistinctState<T>(initial: T, same: (a: T, b: T) => boolean) {
  const [value, setValue] = useState(initial);
  const update = useCallback((next: T) => setValue(held => same(held, next) ? held : next), [same]);
  return [value, update] as const;
}
