import { createContext, type ReactNode } from 'react';
import type { ProductRequest } from '../../dashboard/request';
import type { GatedHour } from '../../dashboard/regionTime';
import type { FamilyPlanContext } from '../plan';
import type { LogKindFilterItem } from './LogKindFilter';

export type PickerList = (items: readonly LogKindFilterItem[]) => ReactNode;

export type PickerContextAtHour = Pick<FamilyPlanContext, 'track' | 'countries' | 'widget'>;

export interface TablePicker {
  request(context: PickerContextAtHour, hour: GatedHour): ProductRequest;
  items(data: unknown, context: PickerContextAtHour, hour: GatedHour, selected?: string): readonly (LogKindFilterItem & { sprite?: string })[];
}

export const PickerContext = createContext<((hour: number, selected: string | undefined, children: PickerList) => ReactNode) | null>(null);
