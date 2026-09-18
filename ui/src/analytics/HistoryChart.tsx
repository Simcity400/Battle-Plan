import { memo, useEffect, useRef, useState } from 'react';
import * as echarts from 'echarts/core';
import type { EChartsCoreOption, EChartsType } from 'echarts/core';
import { LineChart } from 'echarts/charts';
import { DataZoomComponent, GridComponent, TooltipComponent } from 'echarts/components';
import { CanvasRenderer } from 'echarts/renderers';
import type { CountrySeries, CountrySeriesPoint } from '../api';
import { errorMessage } from '../api';
import { Icon } from '../Icons';
import { chartValue, coverageLabels, formatHour, formatMetric, metrics } from './model';
import type { MetricKey } from './model';

echarts.use([LineChart, GridComponent, TooltipComponent, DataZoomComponent, CanvasRenderer]);

interface Props {
  series: CountrySeries;
  metric: MetricKey;
  onSelectHour: (hour: number) => void;
}

interface Observation {
  value: [number, number | null];
  sampleHour?: number;
}

function seriesData(series: CountrySeries, metric: MetricKey) {
  const observations: Observation[] = [];
  const low: Array<[number, number | null]> = [];
  const high: Array<[number, number | null]> = [];
  let previousEnd: number | null = null;

  for (const point of series.points) {
    if (previousEnd != null && point.firstHour > previousEnd + 1) {
      observations.push({ value: [previousEnd + 1, null] });
    }
    previousEnd = point.lastHour;
    if (point.status !== 'recorded' || point.missingMetricHours[metric] > 0) {
      observations.push({ value: [point.firstHour, null] });
      continue;
    }
    const first = point.first;
    const last = point.last;
    if (!first || !last || first.values[metric] == null || last.values[metric] == null) {
      observations.push({ value: [point.firstHour, null] });
      continue;
    }
    const firstValue = chartValue(first.values[metric], series.valueScale, metric);
    const lastValue = chartValue(last.values[metric], series.valueScale, metric);
    observations.push({ value: [first.hour, firstValue], sampleHour: first.hour });
    if (last.hour !== first.hour) {
      observations.push({ value: [last.hour, lastValue], sampleHour: last.hour });
    }
    if (series.downsampled && point.firstHour !== point.lastHour) {
      const minimum = chartValue(point.min[metric], series.valueScale, metric);
      const maximum = chartValue(point.max[metric], series.valueScale, metric);
      // These horizontal segments bound the entire interval. They are not
      // observations and cannot be clicked to invent an hour for an extremum.
      low.push([point.firstHour, minimum], [point.lastHour, minimum], [point.lastHour, null]);
      high.push([point.firstHour, maximum], [point.lastHour, maximum], [point.lastHour, null]);
    }
  }
  return { observations, low, high };
}

function findInterval(points: CountrySeriesPoint[], hour: number): CountrySeriesPoint | undefined {
  let left = 0;
  let right = points.length - 1;
  while (left <= right) {
    const middle = (left + right) >>> 1;
    const point = points[middle];
    if (hour < point.firstHour) right = middle - 1;
    else if (hour > point.lastHour) left = middle + 1;
    else return point;
  }
  return undefined;
}

function tooltip(series: CountrySeries, metric: MetricKey, params: unknown): HTMLElement {
  const entries: unknown[] = Array.isArray(params) ? params : [params];
  const parameter = entries.find((entry) => typeof entry === 'object' && entry !== null && 'axisValue' in entry);
  const hour = parameter && typeof parameter === 'object' && 'axisValue' in parameter ? Number(parameter.axisValue) : NaN;
  const point = Number.isFinite(hour) ? findInterval(series.points, hour) : undefined;
  const container = document.createElement('div');
  container.className = 'chart-tooltip';
  if (!point) {
    container.textContent = 'No retained observation at this hour.';
    return container;
  }

  const title = document.createElement('strong');
  title.textContent = point.firstHour === point.lastHour
    ? `Hour ${formatHour(point.firstHour)}`
    : `Interval ${formatHour(point.firstHour)}–${formatHour(point.lastHour)}`;
  container.append(title);
  const list = document.createElement('dl');
  const row = (label: string, value: string) => {
    const term = document.createElement('dt');
    term.textContent = label;
    const description = document.createElement('dd');
    description.textContent = value;
    list.append(term, description);
  };
  row('Coverage', coverageLabels[point.status]);
  if (point.first) row(`First · ${formatHour(point.first.hour)}`, formatMetric(point.first.values[metric], series.valueScale, metric));
  if (point.last && point.last.hour !== point.first?.hour) row(`Last · ${formatHour(point.last.hour)}`, formatMetric(point.last.values[metric], series.valueScale, metric));
  if (point.firstHour !== point.lastHour) {
    row('Interval low', formatMetric(point.min[metric], series.valueScale, metric));
    row('Interval high', formatMetric(point.max[metric], series.valueScale, metric));
  }
  if (point.gapHours) row('Gap hours', formatHour(point.gapHours));
  if (point.unavailableHours) row('Unavailable hours', formatHour(point.unavailableHours));
  if (point.absentHours) row('Country absent', formatHour(point.absentHours));
  if (point.missingMetricHours[metric]) row('Metric missing', `${formatHour(point.missingMetricHours[metric])} hours`);
  container.append(list);
  if (point.reason || point.firstHour !== point.lastHour) {
    const note = document.createElement('p');
    note.textContent = point.reason ?? 'Extrema belong to this interval. Their exact hours and the intervening path are not retained.';
    container.append(note);
  }
  return container;
}

function HistoryChart({ series, metric, onSelectHour }: Props) {
  const element = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const definition = metrics.find((item) => item.key === metric)!;
  const hasLine = series.points.some((point) => point.status === 'recorded'
    && point.missingMetricHours[metric] === 0 && point.first?.values[metric] != null);

  useEffect(() => {
    if (!element.current || !hasLine) return;
    let chart: EChartsType | undefined;
    let observer: ResizeObserver | undefined;
    let animationFrame = 0;
    let resolution: MediaQueryList | null = null;
    setError(null);

    const clicked = (event: unknown) => {
      if (typeof event !== 'object' || event === null || !('data' in event)) return;
      const datum = event.data;
      if (typeof datum === 'object' && datum !== null && 'sampleHour' in datum && typeof datum.sampleHour === 'number') {
        onSelectHour(datum.sampleHour);
      }
    };
    const resize = () => {
      if (animationFrame) return;
      animationFrame = requestAnimationFrame(() => {
        animationFrame = 0;
        chart?.resize();
      });
    };
    const resolutionChanged = () => setRetry((value) => value + 1);

    try {
      chart = echarts.init(element.current, undefined, { renderer: 'canvas', devicePixelRatio: window.devicePixelRatio || 1 });
      const data = seriesData(series, metric);
      const options: EChartsCoreOption = {
        animation: false,
        backgroundColor: 'transparent',
        textStyle: { fontFamily: 'Segoe UI, system-ui, sans-serif', color: '#a7b0ad' },
        grid: { left: 78, right: 30, top: 32, bottom: 80 },
        tooltip: {
          trigger: 'axis', confine: true, renderMode: 'html', transitionDuration: 0,
          backgroundColor: '#222a29', borderColor: '#4a5850', borderWidth: 1,
          padding: [11, 14], extraCssText: 'box-shadow:0 8px 26px #0005;border-radius:6px;',
          axisPointer: { type: 'line', snap: true, lineStyle: { color: '#697969', type: 'dashed' } },
          formatter: (params: unknown) => tooltip(series, metric, params),
        },
        xAxis: {
          type: 'value', min: series.firstHour, max: series.lastHour,
          minInterval: 1, splitNumber: 5,
          axisLabel: { color: '#8f9b95', fontSize: 10, hideOverlap: true, formatter: (value: number) => formatHour(value) },
          axisLine: { lineStyle: { color: '#414b47' } }, axisTick: { show: false }, splitLine: { show: false },
        },
        yAxis: {
          type: 'value', scale: true, splitNumber: 4,
          axisLabel: {
            color: '#8f9b95', fontSize: 10,
            formatter: (value: number) => `${value.toLocaleString('en-US', { notation: 'compact', maximumFractionDigits: 2 })}${definition.suffix}`,
          },
          axisLine: { show: false }, axisTick: { show: false },
          splitLine: { lineStyle: { color: '#30393a', type: 'dashed' } },
        },
        dataZoom: [
          { type: 'inside', filterMode: 'none', zoomOnMouseWheel: 'ctrl', moveOnMouseWheel: false, throttle: 80 },
          {
            type: 'slider', filterMode: 'none', height: 20, bottom: 18,
            borderColor: '#414c45', backgroundColor: '#19201f', fillerColor: '#52644b33',
            handleStyle: { color: '#8ba17d', borderColor: '#a5ba98' },
            dataBackground: { lineStyle: { color: '#66795e' }, areaStyle: { color: '#3b4938' } },
            selectedDataBackground: { lineStyle: { color: '#9fb894' }, areaStyle: { color: '#65795a' } },
            textStyle: { color: '#9ba794', fontSize: 10 }, showDetail: false, showDataShadow: false,
            brushSelect: false,
          },
        ],
        series: [
          {
            name: 'Recorded observations', type: 'line', data: data.observations,
            smooth: false, connectNulls: false, showSymbol: true,
            symbol: 'circle', symbolSize: 4, lineStyle: { width: 1.8, color: '#b2cda5' },
            itemStyle: { color: '#b2cda5' }, emphasis: { scale: 1.8 },
          },
          {
            name: 'Interval low', type: 'line', data: data.low, smooth: false, connectNulls: false,
            showSymbol: false, silent: true, lineStyle: { width: 1, type: 'dashed', color: '#91a682', opacity: .6 },
          },
          {
            name: 'Interval high', type: 'line', data: data.high, smooth: false, connectNulls: false,
            showSymbol: false, silent: true, lineStyle: { width: 1, type: 'dashed', color: '#91a682', opacity: .6 },
          },
        ],
      };
      chart.setOption(options, { notMerge: true });
      chart.on('click', clicked);
      observer = new ResizeObserver(resize);
      observer.observe(element.current);
      window.addEventListener('resize', resize);
      resolution = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      resolution.addEventListener('change', resolutionChanged);
    } catch (cause) {
      setError(errorMessage(cause));
    }

    return () => {
      if (animationFrame) cancelAnimationFrame(animationFrame);
      observer?.disconnect();
      window.removeEventListener('resize', resize);
      resolution?.removeEventListener('change', resolutionChanged);
      chart?.off('click', clicked);
      chart?.dispose();
    };
  }, [series, metric, definition.suffix, onSelectHour, retry, hasLine]);

  if (!hasLine) return <div className="empty-state compact">
    <h2>No continuous history to draw</h2>
    <p>This metric has unavailable values or mixed coverage throughout this range. Recorded samples and exact interval values remain available below. A smaller analysis range may reveal individual observations.</p>
  </div>;

  return <>
    <div ref={element} className="history-chart" role="img"
      aria-label={`${definition.label} history, engine hours ${formatHour(series.firstHour)} through ${formatHour(series.lastHour)}. Missing data breaks the line. Use the recorded sample selector and exact values table below for keyboard access.`}
      hidden={!!error} />
    {error && <div className="empty-state compact" role="alert">
      <h2>The chart could not be prepared</h2>
      <p className="error-copy">{error}</p>
      <button type="button" className="button" onClick={() => { setError(null); setRetry((value) => value + 1); }}><Icon name="refresh" />Retry chart</button>
    </div>}
  </>;
}

export default memo(HistoryChart);
