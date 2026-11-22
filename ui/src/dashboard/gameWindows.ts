import type { ComponentType } from 'react';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import { dayLabel } from '../charts/draw.ts';

export interface MioPolicySelection {
  kind: 'mioPolicyChooser';
  sourceId: string;
  revision: string;
  branchId: string;
  hour: number;
  organisation: string;
}

export type GameWindowSelection = MioPolicySelection;
export interface GameWindowProps<S extends GameWindowSelection = GameWindowSelection> {
  selection: S;
  active: boolean;
  client: DashboardClient;
  onDiagnostics?: (lines: string[]) => void;
  onShown?: (shown: ShownWindow | null) => void;
}

export interface ShownWindow { hour: number; readHour: number | null }

export const openedOn = (track: Pick<BoundTrack, 'source' | 'branch'>, tag: string) => JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag]);

export function windowReadNote(shown: ShownWindow): string | null {
  return shown.readHour === null || shown.readHour === Math.floor(shown.hour) ? null : `Recorded ${dayLabel(shown.readHour)}`;
}

export function windowDayLabel(shown: ShownWindow): string {
  const note = windowReadNote(shown);
  return note ? `${dayLabel(shown.hour)} · ${note}` : dayLabel(shown.hour);
}

const windows = new Map<GameWindowSelection['kind'], ComponentType<GameWindowProps>>();

export function registerGameWindow<S extends GameWindowSelection>(kind: S['kind'], component: ComponentType<GameWindowProps<S>>) {
  windows.set(kind, component as ComponentType<GameWindowProps>);
}

export const gameWindow = (kind: GameWindowSelection['kind']): ComponentType<GameWindowProps> | null => windows.get(kind) ?? null;

export function withWindowDiagnostics(own: readonly string[], window: readonly string[]): readonly string[] {
  if (!window.length) return own;
  const seen = new Set(own);
  const lines = [...own];
  for (const line of window) if (!seen.has(line)) { seen.add(line); lines.push(line); }
  return lines;
}
