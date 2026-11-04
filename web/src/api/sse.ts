// The server's live event stream, read with fetch so the bearer header travels (EventSource cannot
// send headers). Events: `deal`, `break`, `alm`, `resync`; keep-alive comments every 15 s.
import { endSession, session } from './client';
import type { ServerEvent } from './types';

export type StreamState = 'connected' | 'connecting' | 'disconnected';

export interface StreamHandlers {
  onEvent: (e: ServerEvent) => void;
  onState: (s: StreamState) => void;
  onPing?: () => void;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Opens the stream and keeps it open with backoff until the returned function is called. */
export function openEvents(h: StreamHandlers): () => void {
  let stopped = false;
  let ctrl: AbortController | null = null;
  let delay = 1000;

  const dispatch = (block: string) => {
    let name = 'message';
    const data: string[] = [];
    for (const raw of block.split('\n')) {
      const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
      if (line.startsWith(':')) {
        h.onPing?.();
        continue;
      }
      if (line.startsWith('event:')) name = line.slice(6).trim();
      else if (line.startsWith('data:')) data.push(line.slice(5).trimStart());
    }
    if (name === 'resync' && data.length === 0) {
      h.onEvent({ type: 'resync' });
      return;
    }
    if (!data.length) return;
    try {
      h.onEvent(JSON.parse(data.join('\n')) as ServerEvent);
    } catch {
      // a malformed frame is ignored; the next resync catches anything missed
    }
  };

  const run = async () => {
    while (!stopped) {
      const tok = session.get()?.token;
      if (!tok) {
        h.onState('disconnected');
        return;
      }
      h.onState('connecting');
      ctrl = new AbortController();
      try {
        const res = await fetch('/events', {
          headers: { accept: 'text/event-stream', authorization: `Bearer ${tok}` },
          signal: ctrl.signal,
          cache: 'no-store',
        });
        if (res.status === 401) {
          endSession('Your session ended. Sign in again.');
          return;
        }
        if (!res.ok || !res.body) throw new Error(`stream ${res.status}`);
        h.onState('connected');
        delay = 1000;
        const reader = res.body.getReader();
        const dec = new TextDecoder();
        let buf = '';
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += dec.decode(value, { stream: true });
          let idx: number;
          while ((idx = buf.indexOf('\n\n')) >= 0) {
            const block = buf.slice(0, idx);
            buf = buf.slice(idx + 2);
            dispatch(block);
          }
        }
      } catch {
        if (stopped) return;
      }
      h.onState('disconnected');
      await sleep(delay);
      delay = Math.min(delay * 2, 30_000);
    }
  };

  void run();
  return () => {
    stopped = true;
    ctrl?.abort();
  };
}
