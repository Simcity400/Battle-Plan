import { createContext } from 'react';

export const LiveMotion = createContext<(() => number | undefined) | null>(null);
