import type { ComponentType } from 'react';
import type { DesignerKind } from '../generated/dashboard';
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

export interface DivisionDesignerSelection {
  kind: 'divisionDesigner';
  sourceId: string;
  revision: string;
  branchId: string;
  hour: number;
  tag: string;
  template: string;
  designer: DesignerKind;
}

export interface EquipmentDesignerSelection {
  kind: 'equipmentDesigner';
  sourceId: string;
  revision: string;
  branchId: string;
  hour: number;
  tag: string;
  variant: string;
}

export type GameWindowSelection =
  | MioPolicySelection
  | DivisionDesignerSelection
  | EquipmentDesignerSelection;

export interface GameWindowProps<S extends GameWindowSelection = GameWindowSelection> {
  selection: S;
  active: boolean;
  client: DashboardClient;
  onDiagnostics?: (lines: string[]) => void;
  onShown?: (shown: ShownWindow | null) => void;
  onClose?: () => void;
}

export interface ShownWindow {
  hour: number;
  readHour: number | null;
  title: string | null;
  logicalSize?: readonly [number, number];
}

export interface WindowPosition { readonly left: number; readonly top: number }

interface ActiveWindowSlot {
  ownerId: string;
  position: WindowPosition | null;
  close: () => void;
}

const activeSlots = new Map<GameWindowSelection['kind'], ActiveWindowSlot>();

export function claimGameWindowSlot(
  kind: GameWindowSelection['kind'],
  ownerId: string,
  close: () => void,
): WindowPosition | null {
  const existing = activeSlots.get(kind);
  let inherited: WindowPosition | null = null;
  if (existing && existing.ownerId !== ownerId) {
    inherited = existing.position;
    activeSlots.delete(kind);
    existing.close();
  } else if (existing) {
    inherited = existing.position;
  }
  activeSlots.set(kind, { ownerId, position: inherited, close });
  return inherited;
}

export function updateGameWindowSlotPosition(
  kind: GameWindowSelection['kind'],
  ownerId: string,
  position: WindowPosition,
): void {
  const existing = activeSlots.get(kind);
  if (existing && existing.ownerId === ownerId) {
    existing.position = position;
  }
}

export function releaseGameWindowSlot(
  kind: GameWindowSelection['kind'],
  ownerId: string,
): void {
  const existing = activeSlots.get(kind);
  if (existing && existing.ownerId === ownerId) {
    activeSlots.delete(kind);
  }
}

export const openedOn = (track: Pick<BoundTrack, 'source' | 'branch'>, tag: string) => JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag]);

export function windowReadNote(shown: ShownWindow): string | null {
  return shown.readHour === null || shown.readHour === Math.floor(shown.hour) ? null : `Recorded ${dayLabel(shown.readHour)}`;
}

export function windowDayLabel(shown: ShownWindow): string {
  const note = windowReadNote(shown);
  return note ? `${dayLabel(shown.hour)} · ${note}` : dayLabel(shown.hour);
}

const windows = new Map<GameWindowSelection['kind'], ComponentType<GameWindowProps>>();
const listeners = new Set<() => void>();

export function registerGameWindow<S extends GameWindowSelection>(kind: S['kind'], component: ComponentType<GameWindowProps<S>>) {
  windows.set(kind, component as ComponentType<GameWindowProps>);
  for (const listener of listeners) listener();
}

export function unregisterGameWindow(kind: GameWindowSelection['kind']) {
  if (!windows.delete(kind)) return;
  for (const listener of listeners) listener();
}

export const gameWindow = <K extends GameWindowSelection['kind']>(
  kind: K,
): ComponentType<GameWindowProps<Extract<GameWindowSelection, { kind: K }>>> | null =>
  (windows.get(kind) as ComponentType<GameWindowProps<Extract<GameWindowSelection, { kind: K }>>> | undefined) ?? null;

export function subscribeGameWindows(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function withWindowDiagnostics(own: readonly string[], window: readonly string[]): readonly string[] {
  if (!window.length) return own;
  const seen = new Set(own);
  const lines = [...own];
  for (const line of window) if (!seen.has(line)) { seen.add(line); lines.push(line); }
  return lines;
}
