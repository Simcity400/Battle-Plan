

export interface LedgerGovernment {
  hour: number; readHour: number; country: number; instance: number; epoch: string; sequence: string;
  cause: string; beforeToken: string | null; afterToken: string | null; covered: boolean;
}
