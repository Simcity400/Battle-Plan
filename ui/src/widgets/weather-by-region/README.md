# Weather by region

**What it shows:** Recorded weather by strategic region and day, in the Air superiority timeline's form: each day's conditions (rain, snow, sandstorm and the rest, as the recording names them) as stripes, the region's temperature on a signed ramp, or one recorded weather modifier on a ramp.

**Product(s) it reads:** [`regional-weather`](../../../../crates/dashboard-weather/README.md), a keyed family prepared from the hourly regional weather. Requests ask for daily runs (each day's value is its last reached recorded hour) of the chosen view, in 90-day blocks around the days shown; playback looks the day up locally.

**Settings:** Show (Conditions, Temperature or Modifiers; also the switch above the chart and picker cards), Modifier, Air theatres and Regions (all regions by default), where the breakdown opens; Campaign, Follow, Pin or Range.

**Notable behaviour:**

- A day's cell is the day's last reached recorded hour, never a daily average or a dominant condition: every flag set then shows as a stripe, in the recording's order. No flag set is blank.
- Modifiers are the recorded raw values on their own range; one not witnessed is hatched as unknown, never zero. Days not recorded are shaded; hours still being prepared draw the shared band.
- The tooltip lists every flag and modifier at the day's hour with its read. Jump reaches the read.
- Clicking a region opens its provinces at the day (temperature, snow, mud), following playback or pinned; the provinces are read on demand at that hour.
- Temperature is the average of the region's provinces' reconstructed temperatures, shown only when every province's is known; otherwise the cell stays blank and the tooltip says it is not available.
- Night is not shown: it is not a share of the day in the recording. No generic air penalty is derived.

**Files:** [WeatherWidget.tsx](WeatherWidget.tsx) draws the heat chart; [weatherData.ts](weatherData.ts) validates the reply and builds rows and tooltips; [RegionProvinces.tsx](RegionProvinces.tsx) is the region drill; [useWeather.ts](useWeather.ts) requests the days and lists choices. Days come from [keyedDays.ts](../../dashboard/keyedDays.ts); the chart is the heat kit's [HeatChart](../../charts/heat/HeatChart.tsx).

Shared framework conventions: [widgets](../README.md).
