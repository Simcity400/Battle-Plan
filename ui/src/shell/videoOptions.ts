/** The native maps' display settings as the Map graphics form edits them. */
export interface VideoOptions { useGameSettings: boolean; uiScale: number; vsync: boolean; mapUiScale: number | null; mapFrameLimit: number | null }

/** The fields the form sets. Saved settings also carry the maps' diagnostics switch,
 * which the game's own settings never have and the form never changes. */
const DISPLAY_FIELDS = ['useGameSettings', 'uiScale', 'vsync', 'mapUiScale', 'mapFrameLimit'] as const;

/** Whether two settings display the maps alike, whatever else they carry. */
export const sameDisplay = (first: VideoOptions | null | undefined, second: VideoOptions | null | undefined) =>
  !!first && !!second && DISPLAY_FIELDS.every(field => first[field] === second[field]);
