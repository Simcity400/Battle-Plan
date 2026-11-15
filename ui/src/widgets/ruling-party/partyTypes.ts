

/** One recorded change of government with its execution cause: a row of the politics ledger's `governments` view,
 * which the ruling party widget shows once that family is served ([`governmentChanges`](./governmentData.ts)). */
export interface LedgerGovernment {
  hour: number; readHour: number; country: number; instance: number; epoch: string; sequence: string;
  cause: string; beforeToken: string | null; afterToken: string | null; covered: boolean;
}
