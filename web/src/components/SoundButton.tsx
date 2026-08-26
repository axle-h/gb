import { useCallback, useEffect, useRef, useState } from 'react';
import { AudioPlayer, audioIsSupported } from '../audio';
import type { Connection } from '../api';

const STORAGE_KEY = 'gb.sound';

/**
 * The speaker, bottom-left of the screen.
 *
 * ⚠️ **On the screen rather than in the header**, which is where every video player puts it and
 * where the thing making the noise is. The header is already three lines on a phone — `App`'s
 * `.about` block exists because it ran out of room — and this is not a fact about the run.
 *
 * ⚠️ **Off by default, and the stream is not opened until it is on.** Two reasons that happen to
 * agree: an `AudioContext` starts suspended under every autoplay policy, so a connection opened
 * before a gesture would be decoded into nothing; and a livestream that starts talking to a viewer
 * unbidden is rude. The consequence worth knowing is that sound costs the server nothing at all
 * until someone asks for it — `EmulatorHost::drain_audio` does not even encode.
 */
export function SoundButton() {
  const [supported, setSupported] = useState<boolean | null>(null);
  const [on, setOn] = useState(false);
  // The browser refused to resume without a gesture. Not an error — the button just has to be
  // pressed rather than remembered.
  const [needsGesture, setNeedsGesture] = useState(false);
  const [connection, setConnection] = useState<Connection>('connecting');
  const player = useRef<AudioPlayer | null>(null);
  /** ⚠️ A start in flight is not a player: see `startPlaying`. */
  const starting = useRef(false);

  const stop = useCallback(() => {
    player.current?.stop();
    player.current = null;
  }, []);

  // ⚠️ **`player.current` is set only once the context is genuinely running.** It used to be
  // assigned before the `await`, which meant a start that had not finished — the mount-time one,
  // which has no user activation behind it — read as a working player, and the click that came
  // afterwards returned early and connected nothing. The `starting` latch is what stops a second
  // click stacking a second context on top of the first while that is resolved.
  const startPlaying = useCallback(async () => {
    if (player.current) return true;
    if (starting.current) return false;
    starting.current = true;
    const created = new AudioPlayer('/api/audio', setConnection, (why) => {
      // The server will never answer: audio is off on this deployment, or this build predates it.
      // Either way the control has nothing left to offer, so it goes rather than sitting there
      // failing.
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
      // A stored `on` is a request, not a promise: Chrome honours it when the origin has enough
      // media engagement and otherwise leaves the context suspended, which `start` reports.
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

  // ⚠️ **Nothing at all rather than a disabled button** when the browser cannot decode Opus through
  // WebCodecs (Firefox on Android, Safari before 26) or the server is not serving it. A control that
  // is permanently greyed out is a worse answer than no control: there is nothing the viewer can do
  // about either.
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
