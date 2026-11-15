export type Compression = 'low' | 'high';

/** What Low and High cost, in the words both settings panels use. */
export const COMPRESSION_NOTE = 'Low: the recorder keeps pace with the game. High: a smaller file, and the recorder can fall behind at full speed.';
/** What Auto Compress does, shown only while Compression is Low. */
export const AUTO_COMPRESS_NOTE = 'After saving, compresses the recording to the smaller High size in the background; usable at once meanwhile.';
