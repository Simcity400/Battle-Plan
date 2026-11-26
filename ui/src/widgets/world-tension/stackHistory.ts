import type { WorldTensionStackReply } from '../../generated/dashboard';

type Stack = WorldTensionStackReply;

const joined = new WeakMap<Stack, { history: Stack; reply: Stack }>();

function append<T>(history: T[], edge: T[]): T[] {
  return history.length === 0 ? edge : edge.length === 0 ? history : history.concat(edge);
}

export function tensionStackAt(history: Stack | null, edge: Stack | null): Stack | null {
  if (!edge || edge.pieceStart === undefined) return edge;
  if (!history || history.pieceStart !== edge.pieceStart || history.branchId !== edge.branchId
    || history.downsampled !== edge.downsampled || history.bands.length !== edge.bands.length
    || !!history.other !== !!edge.other) return null;
  const held = joined.get(edge);
  if (held?.history === history) return held.reply;

  const bySlot = new Map(history.bands.map(band => [band.slot, band]));
  const bands: Stack['bands'] = [];
  for (const band of edge.bands) {
    const closed = bySlot.get(band.slot);
    if (!closed) return null;
    bands.push({ ...band,
      holders: append(closed.holders, band.holders),
      values: append(closed.values, band.values), valuesRaw: append(closed.valuesRaw, band.valuesRaw),
      minValues: append(closed.minValues, band.minValues), maxValues: append(closed.maxValues, band.maxValues),
    });
  }
  const { pieceStart, eventPositions = [], ...head } = edge;
  const events = [];
  let old = 0, current = 0;
  for (let i = 0; i < history.events.length + edge.events.length; i++) {
    const event = eventPositions[current] === i ? edge.events[current++] : history.events[old++];
    if (!event) return null;
    events.push(event);
  }
  const reply: Stack = { ...head, bands, events,
    hours: append(history.hours, edge.hours), total: append(history.total, edge.total),
    totalRaw: append(history.totalRaw, edge.totalRaw), sumRaw: append(history.sumRaw, edge.sumRaw),
    missing: append(history.missing, edge.missing),
    other: edge.other && history.other ? { ...edge.other,
      values: append(history.other.values, edge.other.values), valuesRaw: append(history.other.valuesRaw, edge.other.valuesRaw),
    } : null,
  };
  joined.set(edge, { history, reply });
  return reply;
}
