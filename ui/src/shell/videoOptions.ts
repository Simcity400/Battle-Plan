export interface VideoOptions { useGameSettings: boolean; uiScale: number; vsync: boolean; mapUiScale: number | null; mapFrameLimit: number | null }

const DISPLAY_FIELDS = ['useGameSettings', 'uiScale', 'vsync', 'mapUiScale', 'mapFrameLimit'] as const;

export const MIN_FRAME_LIMIT = 1;
export const MAX_FRAME_LIMIT = 1000;
export const FRAME_LIMIT_HINT = `Use ${MIN_FRAME_LIMIT}–${MAX_FRAME_LIMIT}, or leave empty for no limit.`;

export function frameLimitInput(text: string, badInput = false): { value: number | null } | null {
  if (badInput) return null;
  if (!text.trim()) return { value: null };
  const value = Math.round(Number(text));
  return Number.isFinite(value) && value >= MIN_FRAME_LIMIT && value <= MAX_FRAME_LIMIT ? { value } : null;
}

export const sameDisplay = (first: VideoOptions | null | undefined, second: VideoOptions | null | undefined) =>
  !!first && !!second && DISPLAY_FIELDS.every(field => first[field] === second[field]);
