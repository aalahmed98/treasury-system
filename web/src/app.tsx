import { QueryClientProvider } from '@tanstack/react-query';
import { BrowserRouter, Navigate, Route, Routes } from 'react-router';
import { queryClient } from './api/queries';
import { SessionProvider, useSession } from './api/session';
import { LiveProvider } from './api/live';
import { Shell } from './components/shell';
import { Login } from './pages/Login';
import { Overview } from './pages/Overview';
import { Placeholder } from './pages/Placeholder';
import { Blotter } from './pages/Blotter';
import { Breaks } from './pages/Breaks';
import { Ladder } from './pages/Ladder';

function Gate() {
  const { current, canSeeAlm } = useSession();
  if (!current) return <Login />;
  return (
    <LiveProvider>
      <BrowserRouter basename="/app">
        <Shell>
          <Routes>
            <Route index element={<Overview />} />
            <Route path="blotter" element={<Blotter />} />
            <Route path="breaks" element={<Breaks />} />
            <Route path="liquidity" element={<Ladder />} />
            <Route path="alm/:tool?" element={canSeeAlm ? <Placeholder title="ALM" /> : <Navigate to="/" replace />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Routes>
        </Shell>
      </BrowserRouter>
    </LiveProvider>
  );
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <SessionProvider>
        <Gate />
      </SessionProvider>
    </QueryClientProvider>
  );
}
