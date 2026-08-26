// The transport `/api/video` and `/api/audio` share: a chunked binary body carrying `u32`-LE
// length-prefixed messages, read with `fetch` and reconnected for as long as anyone is watching.
//
// Extracted from `video.ts` when audio arrived rather than copied, because the ~90 lines below are
// almost entirely the *subtle* parts — the abort chaining, the silence watchdog, and telling an
// ordinary disconnect apart from a corrupt stream — and each of those carries a ⚠️ in `CLAUDE.md`
// that was paid for once already.
//
// Deliberately DOM-free apart from `fetch` and the streams API: nothing here knows what a message
// means.

import { STALE_MS, type Connection } from './api';

/**
 * The connection *ended* rather than failed, and there is nothing wrong: what the inflater raised is
 * only the truncated deflate stream that closing a healthy one leaves behind.
 *
 * ⚠️ **An ordinary disconnect can only be told apart structurally, never from the exception.**
 * `VideoStream::frame` flushes the connection's encoder after every message and deliberately never
 * *finishes* it, since finishing the stream is the one thing that would end it — so a body that
 * stops carries no final block and no adler trailer, and `DecompressionStream` reports the missing
 * end as bad input rather than as EOF. It raises a bare `TypeError` (Firefox words it "Error in
 * input stream", node gives it no message at all) which is indistinguishable by inspection from a
 * genuinely corrupt stream, and ⚠️ **matching on that wording would be a guess about three
 * engines' private prose**. So the tell is not the failure, it is whether the *source* closed before
 * the inflater complained.
 *
 * ⚠️ **None of that applies to an uncompressed stream**, which is why `readFramedStream` raises this
 * directly there: with no inflater in the pipe, a server that closes simply ends the reader.
 */
export class Disconnected extends Error {
  constructor(url: string) {
    super(`${url} closed mid-stream`);
    this.name = 'Disconnected';
  }
}

/** Why a stream stopped for good, rather than for a moment. */
export type Fatal = 'unavailable' | 'missing';

export interface StreamOptions {
  /** Whether the body is deflated across the connection. `/api/video` yes, `/api/audio` no. */
  inflate: boolean;
  /** What to call it in the console. */
  label: string;
  /** The server said this endpoint will never answer. The retry loop stops and the caller decides. */
  onFatal?: (why: Fatal) => void;
}

/**
 * Turn a response body into the messages it carries.
 *
 * A zero-length message is the keep-alive and yields nothing — which is why `alive` is called with
 * every inflated *chunk* rather than beside the `yield`. ⚠️ **A watchdog fed from the messages this
 * yields would fire on a screen that simply is not moving**: the keep-alive exists precisely for the
 * case where there is no delta to send, and it is the only traffic a paused game produces. The audio
 * stream makes that sharper rather than softer — a run parked on a spent quota emits no packets at
 * all for hours, and the keep-alive is the entire connection.
 *
 * ⚠️ **The compression is part of the protocol, not a `Content-Encoding`.** A declared encoding
 * invites a proxy to decompress and recompress it, which buffers whole messages and shows up as
 * stutter only in production. `DecompressionStream` is native and does the same job here, where we
 * can see it.
 */
export async function* readFramedStream(
  body: ReadableStream<Uint8Array>,
  signal: AbortSignal,
  alive: () => void,
  options: StreamOptions,
): AsyncGenerator<ArrayBuffer> {
  let ended = false;
  let reader: ReadableStreamDefaultReader<Uint8Array>;

  if (options.inflate) {
    // Both pipes carry the signal so an abort reaches the *source*: aborting the reader alone leaves
    // the response body draining in the background until the server notices.
    // The cast is the DOM types being stricter than the spec: `DecompressionStream` accepts any
    // `BufferSource`, which `Uint8Array` is, but the two `WritableStream`s are not assignable.
    const inflating = new DecompressionStream('deflate');
    // The identity tap is the whole of how an ordinary disconnect is recognised: `flush` runs when
    // the *body* closes and never runs when the pipe is aborted or the transport fails, which
    // separates "the connection ended" from "the stream was corrupt" without inspecting a single
    // exception. It is ordered ahead of the failure it explains rather than racing it — closing this
    // transform is what closes the inflater's writable, which is what makes zlib notice it never
    // reached an end.
    const tap = new TransformStream<Uint8Array, Uint8Array>({
      flush() {
        ended = true;
      },
    });
    body
      .pipeThrough(tap, { signal })
      .pipeTo(inflating.writable as WritableStream<Uint8Array>, { signal })
      .catch(() => {});
    reader = (inflating.readable as ReadableStream<Uint8Array>).getReader();
  } else {
    reader = body.getReader();
  }

  // The tail of the last chunk that was not yet a whole message. A message spans chunk boundaries
  // routinely — neither deflate's output nor TCP's has anything to do with our framing.
  let pending = new Uint8Array(0);

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        // With no inflater there is no truncated-stream exception to classify, so an orderly end of
        // body *is* the disconnect and has to be raised as one — otherwise the generator simply
        // returns and the caller reads it as "the stream finished", which this one never does.
        if (!options.inflate) throw new Disconnected(options.label);
        return;
      }
      if (signal.aborted) return;
      alive();
      const merged = new Uint8Array(pending.length + value.length);
      merged.set(pending);
      merged.set(value, pending.length);
      pending = merged;

      let at = 0;
      for (;;) {
        if (pending.length - at < 4) break;
        const length = new DataView(pending.buffer, pending.byteOffset + at, 4).getUint32(0, true);
        if (pending.length - at - 4 < length) break;
        at += 4;
        if (length > 0) {
          yield pending.buffer.slice(pending.byteOffset + at, pending.byteOffset + at + length);
        }
        at += length;
      }
      pending = pending.subarray(at);
    }
  } catch (failure) {
    if (ended) throw new Disconnected(options.label);
    throw failure;
  } finally {
    reader.cancel().catch(() => {});
  }
}

/**
 * Keep a binary stream open for the length of the run, reconnecting for as long as anyone is
 * watching.
 *
 * `EventSource` used to do this part by itself; a `fetch` does not, so the retry is here. It is not
 * a regression in robustness — `EventSource` gives up permanently on some errors and
 * `useEventStream` already had to rebuild it — but it *is* the one thing binary framing costs.
 *
 * ⚠️ **A reconnect of `/api/video` is also the resync**, because every connection opens with a
 * keyframe, which is why `Screen` needs no resync path of its own. Audio has no equivalent and needs
 * none: an Opus packet decodes on its own, so a reconnect costs a listener the jitter buffer and
 * nothing else.
 *
 * ⚠️ **`catch` is only half the story: a body can stall for ever without throwing.** See `STALE_MS`.
 * A dropped network gives this loop nothing to catch, so the watchdog aborts the attempt itself and
 * lets the loop below treat it as any other failure.
 */
export function subscribeFramed(
  url: string,
  onMessage: (message: ArrayBuffer) => void,
  onConnection: (connection: Connection) => void,
  options: StreamOptions,
): () => void {
  const controller = new AbortController();
  const RETRY_MS = 1000;

  (async () => {
    let first = true;
    while (!controller.signal.aborted) {
      // ⚠️ **One controller per attempt, chained to the outer one.** The watchdog has to be able to
      // abandon a connection *without* ending the loop, which aborting the caller's own signal would
      // do — the checks below read it as "the component unmounted" and return.
      const attempt = new AbortController();
      const abandon = () => attempt.abort();
      controller.signal.addEventListener('abort', abandon, { once: true });
      let watchdog: ReturnType<typeof setTimeout> | undefined;
      const alive = () => {
        clearTimeout(watchdog);
        watchdog = setTimeout(abandon, STALE_MS);
      };

      try {
        onConnection(first ? 'connecting' : 'reconnecting');
        const response = await fetch(url, { signal: attempt.signal, cache: 'no-store' });
        // ⚠️ **Two answers mean "stop asking", and they are not the same answer.** A 503 is the
        // feature turned off on a build that has it (`GB_AUDIO_BITRATE=0`); a 404 is a build older
        // than the endpoint. Either way a retry every second for the length of the run is noise the
        // server does not need, so the loop ends and the caller hides the control.
        if (response.status === 503 || response.status === 404) {
          options.onFatal?.(response.status === 503 ? 'unavailable' : 'missing');
          return;
        }
        if (!response.ok || !response.body) throw new Error(`${options.label} answered ${response.status}`);
        onConnection('live');
        first = false;
        alive();
        for await (const message of readFramedStream(response.body, attempt.signal, alive, options)) {
          onMessage(message);
        }
      } catch (failure) {
        if (controller.signal.aborted) return;
        // ⚠️ **A disconnect is normal behaviour and must not be logged as a fault.** The pill
        // already says `reconnecting…`, the next connection repairs whatever it needs to, and
        // nothing is lost — so it goes to a level the console hides by default, and everything else
        // stays loud. A watchdog abort arrives here too, as an `AbortError`, and is one of the loud
        // ones: 8 s of silence on a stream with a 2 s keep-alive is a fault whoever is watching
        // should see.
        if (failure instanceof Disconnected) console.debug(`${options.label} ended, reconnecting`);
        else console.error(`${options.label} dropped, reconnecting`, failure);
      } finally {
        clearTimeout(watchdog);
        controller.signal.removeEventListener('abort', abandon);
      }
      if (controller.signal.aborted) return;
      onConnection('reconnecting');
      await new Promise((resolve) => setTimeout(resolve, RETRY_MS));
    }
  })();

  return () => controller.abort();
}
