export interface PlaybackBounds { readonly first: number; readonly end: number }

export class PlaybackIntervals<T> {
  private readonly hours: ReadonlyArray<number>;
  private readonly read: (hour: number) => T;
  private readonly bounds?: (value: T, hour: number) => PlaybackBounds;
  private held?: PlaybackBounds & { readonly value: T };

  constructor(hours: Iterable<number>, read: (hour: number) => T, bounds?: (value: T, hour: number) => PlaybackBounds) {
    this.read = read; this.bounds = bounds;
    this.hours = [...new Set(hours)].sort((a, b) => a - b);
  }

  get edges(): ReadonlyArray<number> { return this.hours; }

  at(hour: number): T {
    const held = this.held;
    if (held && held.first <= hour && hour < held.end) return held.value;
    let low = 0, high = this.hours.length;
    while (low < high) {
      const middle = Math.floor((low + high) / 2);
      if (this.hours[middle] <= hour) low = middle + 1; else high = middle;
    }
    const value = this.read(hour), bounds = this.bounds?.(value, hour);
    this.held = { first: Math.max(this.hours[low - 1] ?? -Infinity, bounds?.first ?? -Infinity),
      end: Math.min(this.hours[low] ?? Infinity, bounds?.end ?? Infinity), value };
    return value;
  }

  span(hour: number): PlaybackBounds {
    let low = 0, high = this.hours.length;
    while (low < high) {
      const middle = Math.floor((low + high) / 2);
      if (this.hours[middle] <= hour) low = middle + 1; else high = middle;
    }
    return { first: this.hours[low - 1] ?? -Infinity, end: this.hours[low] ?? Infinity };
  }
}

export function* spanChanges(spans: Iterable<Readonly<{ firstHour: number; lastHour: number }>>): Generator<number> {
  for (const span of spans) { yield span.firstHour; yield span.lastHour + 1; }
}
