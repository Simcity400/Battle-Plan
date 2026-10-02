/** Native Project Iterations Completed text, with NUMBER (141ED9730).
 * The reply localizes each count once; this is its only rendering site. */
export default function IterationRow({ text, recorded }: { text: string | null; recorded: boolean }) {
  return <div className="special-project-iterations">{text ?? (recorded ? 'Localisation unavailable' : 'Iterations not recorded')}</div>;
}
