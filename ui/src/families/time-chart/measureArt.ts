import type { ArtHandle } from '../plan.ts';

export const MEASURE_SPRITES = {
  politicalPower: 'GFX_pol_power',
  commandPower: 'GFX_command_power',
  stability: 'GFX_stability_icon',
  warSupport: 'GFX_war_support_icon',
  armyExperience: 'GFX_army_experience',
  navyExperience: 'GFX_navy_experience',
  airExperience: 'GFX_air_experience',
  fuel: 'GFX_fuel_texticon',
  worldTension: 'GFX_world_tension_icon',
  balanceOfPower: 'GFX_power_balance_texticon',
  balanceOfPowerLeft: 'GFX_BoP_left_texticon',
  balanceOfPowerRight: 'GFX_BoP_right_texticon',
  warAlert: 'GFX_global_alert_icons_no_backgrounds',
} as const;

export type ChartMeasureArt = keyof typeof MEASURE_SPRITES;

const MEASURE_FRAMES: Partial<Record<ChartMeasureArt, number>> = { warAlert: 12 };

export function measureArtHandle(measure: ChartMeasureArt): ArtHandle {
  return { kind: 'sprite', key: MEASURE_SPRITES[measure], frame: MEASURE_FRAMES[measure] ?? 0 };
}
