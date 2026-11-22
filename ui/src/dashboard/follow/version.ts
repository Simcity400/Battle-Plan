export interface Version { generation: number; hour: number; sequence: number }

export const newer = (version: Version, than: Version | undefined) => !than || (version.generation !== than.generation
  ? version.generation > than.generation : version.hour !== than.hour ? version.hour > than.hour : version.sequence > than.sequence);
