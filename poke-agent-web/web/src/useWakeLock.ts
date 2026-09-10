import { useEffect } from 'react';

/**
 * Keep a phone's screen on while it is showing the run.
 *
 * This page is a livestream with no input: someone watching a battle play out touches nothing for
 * minutes at a time, and a phone reads that as an idle device and dims itself, then locks. Every
 * other kind of video on a phone holds a wake lock for exactly this reason — a `<video>` element
 * playing gets one from the browser for free, and a canvas fed by `/api/video` gets nothing, so it
 * has to be asked for.
 *
 * ⚠️ **The lock is only ever taken on a coarse pointer**, not on a narrow window. The obvious
 * predicate is the 640px breakpoint the rest of the layout folds at, and it is the wrong one: the
 * question here is about the *device*, not the column count. A desk browser in a half-width window
 * is still a machine whose screensaver and lock timer the viewer chose on purpose and would not
 * thank us for disabling, and a tablet in landscape is over the breakpoint but still dims on you
 * after thirty seconds. `(pointer: coarse)` is the thing actually being asked about, and it is
 * re-evaluated rather than sampled once, because a tablet with a keyboard attached can change its
 * answer under a running page.
 *
 * ⚠️ **A wake lock does not survive the tab being hidden, and does not come back by itself.** The
 * user agent releases the sentinel whenever the document stops being visible — switching apps,
 * locking the phone by hand, a notification pulled down — and re-showing the page restores none of
 * it. So `visibilitychange` is not an optimisation here, it is the whole mechanism: without the
 * re-acquire the lock lasts until the first time you glance at something else and never again, which
 * is worse than not having it, because it works when it is tested and not when it is used.
 *
 * There is no control for it and it is not remembered anywhere. It costs nothing, it is invisible
 * when it works, and it stops the moment the page is not on screen — the state a sound toggle has to
 * be persisted and defaulted-off for (`SoundButton`) does not exist here.
 *
 * A request can be refused, and being refused is ordinary rather than an error: the API is
 * secure-context only, so a phone pointed at `http://192.168.x.x:8080` on the LAN has no
 * `navigator.wakeLock` at all, and Chrome declines the request outright on a low battery. Neither is
 * worth telling a viewer about, and neither is worth retrying in a loop — the next visibility change
 * asks again, and nothing else does.
 */
export function useWakeLock(): void {
  useEffect(() => {
    if (!('wakeLock' in navigator)) return;
    const touch = matchMedia('(pointer: coarse)');

    // `held` is the lock we believe we have. It is cleared from three places: our own release, the
    // sentinel's `release` event (the user agent dropping it on hide, or on a low battery), and the
    // cancelled path below — so `acquire` can use it as the "already covered" test.
    let held: WakeLockSentinel | null = null;
    // ⚠️ Two latches rather than one. `pending` stops StrictMode's double-invoked effect and a burst
    // of visibility changes stacking requests that resolve into each other; `cancelled` catches the
    // request that is still in flight when the effect is torn down, whose sentinel arrives after
    // there is anywhere left to put it and has to be released on the spot rather than leaked.
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
