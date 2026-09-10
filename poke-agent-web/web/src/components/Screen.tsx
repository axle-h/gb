import { useEffect, useReducer, useRef, useState } from 'react';
import type { Connection } from '../api';
import { HEIGHT, VideoDecoder, WIDTH, subscribeVideo } from '../video';

/**
 * The game screen: a 160×144 canvas, CSS-scaled, fed by `/api/video`.
 *
 * Nothing here goes through React state — at 30 fps a `setState` per message would re-render the
 * page thirty times a second to change pixels React does not own. The decoder writes into a buffer
 * the `ImageData` is a view onto, so a frame costs one `putImageData` and no copy.
 */
export function Screen({ pausedUntil }: { pausedUntil: number | null }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [connection, setConnection] = useState<Connection>('connecting');
  // A decode error means our palette or our pixels are suspect, and the server only volunteers a
  // keyframe on a palette reset. Reconnecting is what gets one: `/api/video` opens every connection
  // with the current keyframe (§5.2).
  const [generation, resync] = useReducer((n: number) => n + 1, 0);

  useEffect(() => {
    const context = canvas.current?.getContext('2d');
    if (!context) return;
    const decoder = new VideoDecoder();
    const image = new ImageData(decoder.rgba, WIDTH, HEIGHT);
    let broken = false;

    return subscribeVideo(
      '/api/video',
      (message) => {
        if (broken) return;
        try {
          decoder.apply(message);
        } catch (failure) {
          broken = true;
          console.error('video stream desynchronised, resyncing', failure);
          resync();
          return;
        }
        context.putImageData(image, 0, 0);
      },
      setConnection,
    );
  }, [generation]);

  return (
    <div className="screen">
      <canvas className={pausedUntil ? 'paused' : undefined} ref={canvas} width={WIDTH} height={HEIGHT} />
      {pausedUntil !== null && <PausedOverlay until={pausedUntil} />}
      {connection !== 'live' && <div className="screen-overlay">{connection}…</div>}
    </div>
  );
}

/**
 * A remaining wait, as a viewer would say it. Exported because the header says the same thing, and
 * two roundings of one number is a way for the page to contradict itself.
 *
 * Coarse on purpose: a park is minutes to hours, and a countdown that ticks every second draws the
 * eye to a number nobody is waiting on second by second.
 */
export function describeRemaining(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  const [hours, minutes] = [Math.floor(seconds / 3600), Math.floor((seconds % 3600) / 60)];
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m`;
  return `${seconds}s`;
}

/**
 * What a viewer sees while the run is parked on a spent quota: the last frame dimmed under a PAUSED
 * plate and a live countdown.
 *
 * ⚠️ **The countdown is the one thing on this page driven by a local timer**, and it has to be: the
 * server publishes the deadline once and says nothing more, precisely so that an hours-long wait
 * costs no traffic. The trade is that it is the *viewer's* clock counting down to the server's
 * instant, so a badly-set clock shows a wrong figure; over a wait measured in hours and rendered to
 * the minute, that is not worth a second event stream to fix.
 */
function PausedOverlay({ until }: { until: number }) {
  const [remaining, setRemaining] = useState(() => until - Date.now());

  useEffect(() => {
    setRemaining(until - Date.now());
    const timer = window.setInterval(() => setRemaining(until - Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [until]);

  return (
    <div className="screen-paused">
      <div className="plate">
        <span className="word">PAUSED</span>
        {/* The reason, because a paused game with no explanation reads as a fault. It is the run
            that is waiting, not the page, and nothing is lost while it does. */}
        <span className="why">the model's quota is spent</span>
        <span className="eta">
          {remaining > 0 ? `resumes in ${describeRemaining(remaining)}` : 'resuming…'}
        </span>
      </div>
    </div>
  );
}
