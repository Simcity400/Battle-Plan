import type { AuditRow } from '../draw/notifications';

declare const familyAuditRowBrand: unique symbol;
export type FamilyAuditRow = AuditRow & { readonly [familyAuditRowBrand]: true };

export function auditRow(row: string, value: string): FamilyAuditRow {
  return { row, key: row, value } as FamilyAuditRow;
}

export function auditCell(row: string, cell: string, value: string): FamilyAuditRow {
  return { row, key: `${row}:${cell}`, value } as FamilyAuditRow;
}
