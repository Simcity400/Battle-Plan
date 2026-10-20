import type { WidgetRenderer } from '../../dashboard/definitions';
import WeatherWidget from './WeatherWidget';
import { useWeatherChoices } from './useWeather';
export default { render: WeatherWidget, useOptionChoices: useWeatherChoices } satisfies WidgetRenderer;
