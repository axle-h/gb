import { useCallback, useEffect, useRef, useState } from 'react';
import { AudioPlayer, audioIsSupported } from '../audio';
import type { Connection } from '../api';

const STORAGE_KEY = 'gb.sound';

// The speaker, in the header and kept on phones. Off by default, and the stream is not opened until it
// is on: a context starts suspended before a gesture, and the server encodes nothing without a listener.
export function SoundButton() {
  const [supported, setSupported] = useState<boolean | null>(null);
  const [on, setOn] = useState(false);
  // The browser would not resume without a gesture, so the button has to be pressed.
  const [needsGesture, setNeedsGesture] = useState(false);
  const [connection, setConnection] = useState<Connection>('connecting');
  const player = useRef<AudioPlayer | null>(null);
  const starting = useRef(false);

  const stop = useCallback(() => {
    player.current?.stop();
    player.current = null;
  }, []);

  // `player.current` is set only once the context runs; `starting` stops a second click stacking a second context.
  const startPlaying = useCallback(async () => {
    if (player.current) return true;
    if (starting.current) return false;
    starting.current = true;
    const created = new AudioPlayer('/api/audio', setConnection, (why) => {
      // The server will never answer, so the control goes.
      console.debug(`audio is ${why}`);
      setSupported(false);
      stop();
      setOn(false);
    });
    try {
      const started = await created.start();
      if (!started) {
        setNeedsGesture(true);
        return false;
      }
      player.current = created;
      setNeedsGesture(false);
      return true;
    } finally {
      starting.current = false;
    }
  }, [stop]);

  useEffect(() => {
    let cancelled = false;
    void audioIsSupported().then((can) => {
      if (cancelled) return;
      setSupported(can);
      // A stored `on` is a request: without enough media engagement the context stays suspended.
      if (can && localStorage.getItem(STORAGE_KEY) === 'on') {
        void startPlaying().then((started) => {
          if (!cancelled && started) setOn(true);
        });
      }
    });
    return () => {
      cancelled = true;
      stop();
    };
  }, [startPlaying, stop]);

  const toggle = async () => {
    if (on) {
      stop();
      setOn(false);
      localStorage.setItem(STORAGE_KEY, 'off');
      return;
    }
    if (await startPlaying()) {
      setOn(true);
      localStorage.setItem(STORAGE_KEY, 'on');
    }
  };

  // No control at all, rather than a disabled one, when Opus via WebCodecs or the endpoint is unavailable.
  if (supported !== true) return null;

  const label = on ? (connection === 'live' ? 'sound on' : `sound ${connection}…`) : 'sound off';
  return (
    <button
      className={`sound ${on ? 'on' : ''}`}
      onClick={() => void toggle()}
      title={needsGesture ? 'tap for sound' : label}
      aria-label={label}
      aria-pressed={on}
    >
      {on ? '🔊' : '🔇'}
    </button>
  );
}
