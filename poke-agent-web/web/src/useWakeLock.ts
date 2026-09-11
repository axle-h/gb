import { useEffect } from 'react';

// Keep a phone's screen on while the run is visible: only on a coarse pointer, re-evaluated live, and
// re-acquired on every `visibilitychange`, since the browser releases the lock whenever the tab hides.
export function useWakeLock(): void {
  useEffect(() => {
    if (!('wakeLock' in navigator)) return;
    const touch = matchMedia('(pointer: coarse)');

    // The lock we believe we hold; cleared by our release, the sentinel's `release` event and teardown.
    let held: WakeLockSentinel | null = null;
    // `pending` stops stacked requests; `cancelled` releases a sentinel that resolves after teardown.
    let pending = false;
    let cancelled = false;

    const drop = () => {
      const sentinel = held;
      held = null;
      void sentinel?.release().catch(() => {});
    };

    const acquire = async () => {
      if (held || pending || cancelled) return;
      pending = true;
      try {
        const sentinel = await navigator.wakeLock.request('screen');
        if (cancelled || !touch.matches || document.visibilityState !== 'visible') {
          void sentinel.release().catch(() => {});
          return;
        }
        sentinel.addEventListener('release', () => {
          if (held === sentinel) held = null;
        });
        held = sentinel;
      } catch (err) {
        // Refused: insecure origin, low battery, or a policy of the browser's own. Ordinary.
        console.debug('screen wake lock refused', err);
      } finally {
        pending = false;
      }
    };

    const sync = () => {
      if (touch.matches && document.visibilityState === 'visible') void acquire();
      else drop();
    };

    document.addEventListener('visibilitychange', sync);
    touch.addEventListener('change', sync);
    sync();

    return () => {
      cancelled = true;
      document.removeEventListener('visibilitychange', sync);
      touch.removeEventListener('change', sync);
      drop();
    };
  }, []);
}
