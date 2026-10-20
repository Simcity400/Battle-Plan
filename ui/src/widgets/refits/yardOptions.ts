import type { OptionField } from '../../dashboard/options';
import { TABLE_OPTIONS } from '../../charts/table/options.ts';

/** Shared settings of the naval yards tables: the table's own look and narrow behaviour. */
export const YARD_OPTIONS: readonly OptionField[] = TABLE_OPTIONS.filter(field => field.key !== 'rankMarkers');
