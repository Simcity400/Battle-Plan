import { useId, useLayoutEffect } from 'react';
import type { RefObject } from 'react';

/** Full row models belong to the parity build; ordinary builds expose no row data. */
export interface ParityList {
  kind: 'windowed' | 'table' | 'log' | 'chart';
  label: string;
  complete: boolean;
  total: number;
  rows: unknown[];
  [key: string]: unknown;
}

export interface ParityPreparation {
  /** A parent keeps one identity while its empty state gains a log or table. */
  id?: string;
  prepare: () => void;
  ready: () => boolean;
  error?: () => string | null;
}
type ParityProvider = (() => ParityList) & Partial<ParityPreparation> & { id: string };

declare global {
  interface Window {
    __hoi4ParityLists?: Map<Element, ParityProvider>;
  }
}

/** The build mode cannot change during a component's lifetime. The production
 * branch has no effect, registry, DOM attributes or history serialization. */
export function useParityRows(root: RefObject<Element | null> | Element | null, read: () => ParityList, preparation?: Partial<ParityPreparation>): void {
  if (import.meta.env.MODE === 'parity') {
    const generatedId = useId();
    const id = preparation?.id ?? generatedId;
    useLayoutEffect(() => {
      const element = root && 'current' in root ? root.current : root;
      if (!element) return;
      const registry = window.__hoi4ParityLists ??= new Map();
      const provider: ParityProvider = Object.assign(() => ({ ...read(), id }), { id }, preparation);
      registry.set(element, provider);
      return () => {
        if (registry.get(element) === provider) registry.delete(element);
      };
    }, [root, read, id, preparation]);
  }
}
