// The seam between widgets and the game-window renderer: a widget asks for a game
// window by a semantic selection (recording, branch, hour and native identities),
// never by window paths or copied layouts. The renderer registers each window it
// can draw (components/gameWindowRegistry.tsx); until one is registered the widget
// keeps the row action disabled.
import type { ComponentType } from 'react';
import type { DashboardClient } from './client';
import { dayLabel } from '../charts/draw.ts';

/** An industrial organisation's policy chooser at one recorded hour. */
export interface MioPolicySelection {
  kind: 'mioPolicyChooser';
  sourceId: string;
  revision: string;
  branchId: string;
  /** The playback hour the window describes. */
  hour: number;
  /** The full organisation reference, as the decimal u64 every product spells it;
   * the window resolves its holder from the recorded organisation at `hour`. */
  organisation: string;
}

export type GameWindowSelection = MioPolicySelection;
/** `client` requests the window's own products for the widget that opened it;
 * `onDiagnostics` receives the window's own diagnostics (refused controls,
 * unsupported effects, unavailable artwork), which the widget reports beside its own. */
export interface GameWindowProps<S extends GameWindowSelection = GameWindowSelection> {
  selection: S;
  active: boolean;
  client: DashboardClient;
  onDiagnostics?: (lines: string[]) => void;
  /** What the drawn window answers, for the host to date it; null while none is drawn. */
  onShown?: (shown: ShownWindow | null) => void;
}

/** The hour a drawn window answers, and the hour its sampled facts were read at (null when not said). */
export interface ShownWindow { hour: number; readHour: number | null }

/** The note a window's facts need when they were read before the hour it answers. */
export function windowReadNote(shown: ShownWindow): string | null {
  return shown.readHour === null || shown.readHour === Math.floor(shown.hour) ? null : `Recorded ${dayLabel(shown.readHour)}`;
}

/** A host's day label for a drawn window: the hour it answers, and when its facts were read if earlier. */
export function windowDayLabel(shown: ShownWindow): string {
  const note = windowReadNote(shown);
  return note ? `${dayLabel(shown.hour)} · ${note}` : dayLabel(shown.hour);
}

const windows = new Map<GameWindowSelection['kind'], ComponentType<GameWindowProps>>();

/** Registers the renderer's component for a window kind. */
export function registerGameWindow<S extends GameWindowSelection>(kind: S['kind'], component: ComponentType<GameWindowProps<S>>) {
  windows.set(kind, component as ComponentType<GameWindowProps>);
}

/** The registered component for a window kind, or null while none is. */
export const gameWindow = (kind: GameWindowSelection['kind']): ComponentType<GameWindowProps> | null => windows.get(kind) ?? null;

/** A widget's diagnostics: its own warnings, then the open game window's (refused controls,
 * unsupported effects, unavailable artwork), each once. */
export function withWindowDiagnostics(own: readonly string[], window: readonly string[]): readonly string[] {
  if (!window.length) return own;
  const seen = new Set(own);
  const lines = [...own];
  for (const line of window) if (!seen.has(line)) { seen.add(line); lines.push(line); }
  return lines;
}
