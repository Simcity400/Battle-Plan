import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS } from '../../charts/table/options.ts';

export const YARD_OPTIONS: readonly OptionField[] = TABLE_OPTIONS.filter(field => field.key !== 'rankMarkers');
