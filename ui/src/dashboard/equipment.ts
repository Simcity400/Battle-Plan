import type { OptionField } from './options';

export const REFITS_OPTION: OptionField = {
  key: 'refits', section: 'options', label: 'Count refitted ships',
  hint: 'A refit rebuilds a ship the country already had as another design', kind: 'toggle', default: false,
};

export const unresolvedTypeName = (_key: string) => 'Equipment type name unavailable';
