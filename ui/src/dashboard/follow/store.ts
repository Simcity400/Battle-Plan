import { createContext } from 'react';

export const FollowedWidgetKey = createContext<string | null>(null);
export const FollowedWidgetKind = createContext<string | null>(null);
export const FollowedCard = createContext<Element | null>(null);
