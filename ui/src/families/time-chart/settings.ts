import type { OptionValue } from '../../dashboard/options';
import { powerDecimals } from '../../dashboard/powerDecimals.ts';
import type { ChartYAxisSpec, TimeChartModel } from './model.ts';

export interface LineChartSettings {
  readonly windowHours: number | null;
  readonly yMin: number | null;
  readonly yMax: number | null;
  readonly log: boolean;
}

const finite = (value: OptionValue | undefined): number | null => (typeof value === 'number' && Number.isFinite(value) ? value : null);

export function lineChartSettings(options: Readonly<Record<string, OptionValue>>): LineChartSettings {
  const days = finite(options.windowDays);
  return {
    windowHours: options.playback === 'window' ? Math.max(1, Math.round((days ?? 365) * 24)) : null,
    yMin: finite(options.yMin),
    yMax: finite(options.yMax),
    log: options.log === true,
  };
}

export const DEFAULT_LINE_SETTINGS: LineChartSettings = { windowHours: null, yMin: null, yMax: null, log: false };

export function chartWindow(settings: LineChartSettings, hour: number, extentStart: number): TimeChartModel['view'] {
  return settings.windowHours === null ? undefined : { x: [Math.max(extentStart, hour - settings.windowHours), hour] };
}

export function chartAxis(settings: LineChartSettings): Pick<ChartYAxisSpec, 'min' | 'max' | 'log'> {
  return {
    ...(settings.yMin !== null ? { min: settings.yMin } : {}),
    ...(settings.yMax !== null ? { max: settings.yMax } : {}),
    ...(settings.log ? { log: true } : {}),
  };
}

export interface PowerChartSettings extends LineChartSettings {
  readonly decimals: number;
  readonly endLabels: boolean;
  readonly area: boolean;
}

export function powerChartSettings(options: Readonly<Record<string, OptionValue>>): PowerChartSettings {
  return {
    ...lineChartSettings(options),
    decimals: powerDecimals(options),
    endLabels: options.endLabels !== false,
    area: options.areaFill !== false,
  };
}

export const DEFAULT_POWER_SETTINGS: PowerChartSettings = powerChartSettings({});
