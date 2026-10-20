import { useEffect, useState } from 'react';
import type { Moment } from '../api';
import { capitalise, momentIcon } from './Conversation';

/** The run's turning points, newest first, each with the screen it was taken on. */
export function Highlights({ seen }: { seen: number }) {
  const [moments, setMoments] = useState<Moment[] | null>(null);
  const [failed, setFailed] = useState(false);

  // `seen` is how many moments the log has carried, so a new one refetches the list.
  useEffect(() => {
    let cancelled = false;
    fetch('/api/moments')
      .then((response) => (response.ok ? response.json() : Promise.reject(response.status)))
      .then((list: Moment[]) => {
        if (!cancelled) setMoments(list);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [seen]);

  return (
    <div className="highlights">
      {failed && <p className="dim">the highlights could not be read</p>}
      {!failed && moments === null && <p className="dim">reading the highlights…</p>}
      {!failed && moments?.length === 0 && <p className="dim">nothing worth remembering yet</p>}
      {moments && moments.length > 0 && (
        <ol className="moments">
          {[...moments].reverse().map((moment) => (
            <li key={moment.n} className={moment.kind}>
              <img src={`/api/moments/${moment.n}/screen.png`} alt="" width={160} height={144} loading="lazy" />
              <span className="title">
                {momentIcon(moment.kind)} {capitalise(moment.title)}
              </span>
              <span className="dim">
                {moment.map} · {moment.playtime}
              </span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
