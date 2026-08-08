import { useEffect, useReducer, useRef, useState } from 'react';
import type { Connection } from '../api';
import { subscribe } from '../useEventStream';
import { HEIGHT, VideoDecoder, WIDTH, base64ToBytes } from '../video';

/**
 * The game screen: a 160×144 canvas, CSS-scaled, fed by `/api/video`.
 *
 * Nothing here goes through React state — at 30 fps a `setState` per message would re-render the
 * page thirty times a second to change pixels React does not own. The decoder writes into a buffer
 * the `ImageData` is a view onto, so a frame costs one `putImageData` and no copy.
 */
export function Screen() {
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

    return subscribe(
      '/api/video',
      (data) => {
        if (broken) return;
        try {
          decoder.apply(base64ToBytes(data));
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
      <canvas ref={canvas} width={WIDTH} height={HEIGHT} />
      {connection !== 'live' && <div className="screen-overlay">{connection}…</div>}
    </div>
  );
}
