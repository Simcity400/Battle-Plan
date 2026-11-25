import { createContext, type ReactNode } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { GatedHour } from '../../dashboard/regionTime';
import type { FamilyPlanContext } from '../plan';
import type { LogKindFilterItem } from './LogKindFilter';

export type PickerList = (items: readonly LogKindFilterItem[]) => ReactNode;

export interface PickerProps {
  readonly runtime: WidgetRuntime;
  readonly context: Pick<FamilyPlanContext, 'track' | 'countries' | 'widget'>;
  readonly hour: GatedHour;
  readonly selected?: string;
  readonly children: PickerList;
}

export const PickerContext = createContext<((hour: number, selected: string | undefined, children: PickerList) => ReactNode) | null>(null);
