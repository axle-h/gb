import { useState } from 'react';

/**
 * `POST /api/new-run` — start the game over without restarting the process.
 *
 * ⚠️ **The token is never shipped with the page.** This SPA is served to anyone who loads the site,
 * so a build-time token would be a public one. It is typed in on the first use of the button and
 * kept in `sessionStorage`, which is per-tab and does not survive the tab closing — the shortest
 * lifetime that still means one prompt rather than one per click.
 *
 * A 404 means the server has no `GB_ADMIN_TOKEN` at all and the button cannot work here; a 403 means
 * the stored token is wrong, so it is discarded rather than re-sent on every subsequent click.
 */
const TOKEN_KEY = 'gb-admin-token';

type State = { kind: 'idle' } | { kind: 'working' } | { kind: 'said'; message: string; bad: boolean };

export function NewRunButton() {
  const [state, setState] = useState<State>({ kind: 'idle' });

  async function start() {
    // Destructive and one-way from the viewer's side, so it asks — the current run is checkpointed
    // and kept, but it is not what the page will be showing a second later.
    if (!confirm('Start the game again from the beginning?\n\nThe current run is checkpointed and left on disk, but this page will follow the new one.')) {
      return;
    }
    let token = sessionStorage.getItem(TOKEN_KEY);
    if (!token) {
      token = prompt('Admin token (GB_ADMIN_TOKEN)');
      if (!token) return;
      sessionStorage.setItem(TOKEN_KEY, token);
    }

    setState({ kind: 'working' });
    try {
      const response = await fetch('/api/new-run', { method: 'POST', headers: { 'X-GB-Token': token } });
      const body = await response.json().catch(() => ({}));
      if (response.ok) {
        setState({ kind: 'said', message: `new run ${body.run_id ?? ''}`.trim(), bad: false });
      } else {
        if (response.status === 403) sessionStorage.removeItem(TOKEN_KEY);
        setState({ kind: 'said', message: body.error ?? `HTTP ${response.status}`, bad: true });
      }
    } catch (failure) {
      setState({ kind: 'said', message: `${failure}`, bad: true });
    }
    setTimeout(() => setState({ kind: 'idle' }), 6000);
  }

  return (
    <span className="new-run">
      {state.kind === 'said' && <span className={state.bad ? 'said bad' : 'said'}>{state.message}</span>}
      <button onClick={start} disabled={state.kind === 'working'} title="Start the game again from the beginning">
        {state.kind === 'working' ? 'starting…' : 'new run'}
      </button>
    </span>
  );
}
