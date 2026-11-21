export interface VideoOptions { useGameSettings: boolean; uiScale: number; vsync: boolean; mapUiScale: number | null; mapFrameLimit: number | null }

const DISPLAY_FIELDS = ['useGameSettings', 'uiScale', 'vsync', 'mapUiScale', 'mapFrameLimit'] as const;

export const sameDisplay = (first: VideoOptions | null | undefined, second: VideoOptions | null | undefined) =>
  !!first && !!second && DISPLAY_FIELDS.every(field => first[field] === second[field]);
