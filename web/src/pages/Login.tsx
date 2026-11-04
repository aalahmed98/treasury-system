// The sign-in page: the wordmark, one card, two fields and the primary action.
import { useState, type FormEvent } from 'react';
import { ApiError, sessionNotice } from '../api/client';
import { useSession } from '../api/session';

export function Login() {
  const { login } = useSession();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const notice = sessionNotice;
  const secure = window.location.protocol === 'https:';
  const dev = window.location.hostname === 'localhost' || window.location.hostname === '127.0.0.1';
  const ready = username.trim() !== '' && password !== '' && !busy;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      await login(username.trim(), password);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : 'The server could not be reached.');
      setBusy(false);
    }
  };

  return (
    <div className="login">
      <div className="wordmark">
        <span className="brand">NBB</span>
        <span>Treasury</span>
      </div>
      <div className="t-muted">Positions, reconciliation, liquidity and ALM</div>
      <form className="card" onSubmit={submit} noValidate>
        <label className="field">
          <span className="t-label">Username</span>
          <input className="input input-lg" name="username" autoComplete="username" autoFocus value={username} onChange={(e) => setUsername(e.target.value)} />
        </label>
        <label className="field" style={{ marginTop: 8 }}>
          <span className="t-label">Password</span>
          <input className="input input-lg" type="password" name="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} />
        </label>
        <button type="submit" className="btn btn-primary btn-wide" disabled={!ready} style={{ marginTop: 16 }}>
          {busy ? 'Signing in…' : 'Sign in'}
        </button>
        {error && (
          <div className="notice bad" role="alert">
            {error}
          </div>
        )}
        {notice && !error && <div className="notice warn">{notice}</div>}
        <hr className="hr" style={{ marginTop: 14 }} />
        <div className="t-small" title={secure ? 'TLS' : 'plain HTTP (loopback only)'}>
          {secure ? '🔒' : '⚠'} {window.location.origin}
        </div>
      </form>
      {dev && <div className="t-small" style={{ marginTop: 16 }}>dev server · seeded users fx1, fx2, mm1, alm1, head1 · password "password"</div>}
    </div>
  );
}
