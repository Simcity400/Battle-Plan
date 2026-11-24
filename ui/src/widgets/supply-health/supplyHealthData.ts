export const STATES = ['noSupply', 'strained', 'inSupply'] as const;
export type SupplyState = (typeof STATES)[number];
export const STATE_LABELS = { noSupply: 'No supply', strained: 'Strained', inSupply: 'In supply' } as const;
export const SHARE_MEASURES = ['noSupplyShare', 'strainedShare', 'inSupplyShare'] as const;
export const SUPPLY_FAMILY = 'supply-health';
