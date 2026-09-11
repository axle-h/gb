import { useEffect, useReducer, useRef, useState } from 'react';
import type { Connection } from '../api';
import { HEIGHT, VideoDecoder, WIDTH, subscribeVideo } from '../video';

/** The 160x144 canvas fed by `/api/video`, kept out of React state: a frame is one `putImageData`. */
export function Screen({ pausedUntil }: { pausedUntil: number | null }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [connection, setConnection] = useState<Connection>('connecting');
  // A decode error makes the palette suspect; reconnecting brings a fresh keyframe.
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

/** A remaining wait, coarse on purpose; shared with the header so the two never disagree. */
export function describeRemaining(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  const [hours, minutes] = [Math.floor(seconds / 3600), Math.floor((seconds % 3600) / 60)];
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m`;
  return `${seconds}s`;
}

/** The parked screen: the last frame dimmed under a PAUSED plate, counted down on the viewer's clock. */
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
        <span className="why">the model's endpoint is refusing requests</span>
        <span className="eta">
          {remaining > 0 ? `resumes in ${describeRemaining(remaining)}` : 'resuming…'}
        </span>
      </div>
    </div>
  );
}
