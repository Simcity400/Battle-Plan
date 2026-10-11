/** Project iteration counts use the content text.
 * The reply localizes each count once; this is its only rendering site. */
export default function IterationRow({ text, recorded }: { text: string | null; recorded: boolean }) {
  return <div className="special-project-iterations">{text ?? (recorded ? 'Iteration count unavailable' : 'Iterations not recorded')}</div>;
}
