// The transport `/api/video` and `/api/audio` share: a chunked body of `u32`-LE length-prefixed
// messages, read with `fetch` and reconnected while anyone is watching.

import { STALE_MS, type Connection } from './api';

// The body closed rather than failed. An unfinished deflate stream reports its end as a bare error,
// so a disconnect is recognised by the source closing first, never by the exception's wording.
export class Disconnected extends Error {
  constructor(url: string) {
    super(`${url} closed mid-stream`);
    this.name = 'Disconnected';
  }
}

export type Fatal = 'unavailable' | 'missing';

export interface StreamOptions {
  /** Whether the body is deflated across the connection: video yes, audio no. */
  inflate: boolean;
  label: string;
  /** The server said this endpoint will never answer; the retry loop stops. */
  onFatal?: (why: Fatal) => void;
}

// Turn a response body into its messages. `alive` is fed per chunk, not per message: the zero-length
// keep-alive yields nothing and is the only traffic a paused game sends.
export async function* readFramedStream(
  body: ReadableStream<Uint8Array>,
  signal: AbortSignal,
  alive: () => void,
  options: StreamOptions,
): AsyncGenerator<ArrayBuffer> {
  let ended = false;
  let reader: ReadableStreamDefaultReader<Uint8Array>;

  if (options.inflate) {
    // Both pipes carry the signal so an abort reaches the source. The tap's `flush` runs only when the
    // body closes, which is how an ordinary disconnect is told from a corrupt stream.
    const inflating = new DecompressionStream('deflate');
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

  let pending = new Uint8Array(0);

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        // With no inflater, an orderly end of body is the disconnect.
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

// Keep a binary stream open, reconnecting while anyone watches; a video reconnect is also its resync,
// as every connection opens with a keyframe. A stalled body throws nothing, so a watchdog aborts it.
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
      // One controller per attempt, so the watchdog can abandon a connection without ending the loop.
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
        // 503 is the feature turned off and 404 a build without the endpoint: stop asking.
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
        // A disconnect is normal and logged quietly; a watchdog abort arrives as an `AbortError` and stays loud.
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
