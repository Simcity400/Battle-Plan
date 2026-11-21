import './drill.css';

export default function FollowControl({ following, onFollow, onPause }: {
  following: boolean; onFollow: () => void; onPause?: () => void;
}) {
  return <button type="button" className="button small quiet drill-follow" aria-label="Follow playback" aria-pressed={following} disabled={following && !onPause}
    onClick={following && onPause ? onPause : onFollow}>Follow</button>;
}
