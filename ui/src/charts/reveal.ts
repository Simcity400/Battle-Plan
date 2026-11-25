export interface RevealClock {
  hour(): number;
  playing(): boolean;
  subscribe(listener: () => void): () => void;
}
