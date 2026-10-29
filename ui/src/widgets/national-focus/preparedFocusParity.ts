import type { DashboardSource } from '../../dashboard/client';
import type { Artwork, FocusEntry } from '../../generated/dashboard';
import { loadPreparedFocusHour, loadPreparedFocusPage, loadPreparedFocusDetail, loadPreparedFocusStaticText, preparedArtKey } from './preparedFocusData.ts';
import type { FocusPreparedClient, PreparedFocusArt, PreparedFocusSnapshot } from './preparedFocusData.ts';
import { focusParityModel } from './focusParityModel.ts';

/** Complete model gathering is invoked only by the parity preparation handshake. */
export async function gatherPreparedFocusParity(client: FocusPreparedClient, source: DashboardSource,
  branchId: string, tag: string, hour: number, moment: boolean, signal: AbortSignal, openedOrdinals: readonly number[] = [], snapshot?: PreparedFocusSnapshot) {
  const visible = snapshot?.displayHour === hour ? snapshot : undefined;
  const row = visible?.hour ?? await loadPreparedFocusHour(client, source, branchId, tag, hour, signal);
  const completed: FocusEntry[] = [], artwork = new Map<string, PreparedFocusArt>();
  // National and continuous static text stay apart: one id can name a focus in each.
  const staticByToken = new Map(visible?.staticText.map(item => [item.token, item]));
  const continuousByToken = new Map<string, Artwork>(visible?.continuousStaticText.map(item => [item.token, item]));
  const openedKeys = new Set<string>();
  if (row) {
    const count = BigInt(row.completed.count);
    for (let page = 0n; page < (count || 1n); page++) {
      if (signal.aborted) throw new DOMException('Focus parity cancelled', 'AbortError');
      const data = visible?.page === page ? visible : await loadPreparedFocusPage(client, source, branchId, tag, hour, row, page, signal);
      completed.push(...data.order);
      for (const [key, item] of data.art) artwork.set(key, item);
      for (const item of data.staticText) staticByToken.set(item.token, item);
      for (const item of data.continuousStaticText) continuousByToken.set(item.token, item);
    }
    if (BigInt(completed.length) !== BigInt(row.completed.rows)) throw new Error('Prepared focus parity omitted completions.');
  }
  const tokens = [...new Set([...completed.map(entry => entry.token), ...(row?.current ? [row.current] : [])])].sort();
  for (const item of (await loadPreparedFocusStaticText(client, source, 'national', tokens.filter(token => !staticByToken.has(token)), signal)).text) staticByToken.set(item.token, item);
  const continuous = row?.continuous && !continuousByToken.has(row.continuous) ? [row.continuous] : [];
  for (const item of (await loadPreparedFocusStaticText(client, source, 'continuous', continuous, signal)).text) continuousByToken.set(item.token, item);
  if (row) for (const openedOrdinal of openedOrdinals) {
    const entry = completed.find(item => item.ordinal === openedOrdinal);
    if (entry) {
      const openedKey = preparedArtKey('national', entry.completedByHour ?? row.orderFirstHour ?? row.at, entry.token);
      openedKeys.add(openedKey);
      const detail = visible?.details?.get(openedOrdinal)
        ?? await loadPreparedFocusDetail(client, source, branchId, tag, row, entry, signal);
      const display = artwork.get(openedKey);
      if (detail && display) artwork.set(openedKey, { ...display,
        artwork: { ...display.artwork, description: detail.artwork.description }, reason: detail.reason ?? display.reason });
    }
  }
  if (signal.aborted) throw new DOMException('Focus parity cancelled', 'AbortError');
  return focusParityModel(row && { completed, current: row.current, continuous: row.continuous, paused: row.paused },
    row && moment ? { progress: BigInt(row.progressRaw), dailyGain: row.dailyGainRaw === null ? null : BigInt(row.dailyGainRaw) } : null,
    row?.orderFirstHour ?? undefined, hour, moment, [...staticByToken.values()], [...continuousByToken.values()], (database, token, date) => {
      const key = preparedArtKey(database, date, token), item = artwork.get(key);
      const fallback = (database === 'national' ? staticByToken : continuousByToken).get(token);
      return { artwork: item ? { ...item.artwork, description: item.artwork.description ?? fallback?.description ?? null } : fallback ?? null,
        pending: false, reason: item?.reason ?? null, descriptionDemanded: openedKeys.has(key) };
    });
}
