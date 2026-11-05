import { lazy, Suspense } from 'react';
import { QueryClientProvider } from '@tanstack/react-query';
import { BrowserRouter, Navigate, Route, Routes } from 'react-router';
import { queryClient } from './api/queries';
import { SessionProvider, useSession } from './api/session';
import { LiveProvider } from './api/live';
import { Shell, PageSkeleton } from './components/shell';
import { ToastProvider } from './components/feedback';
import { PaletteProvider } from './components/palette';
import { Login } from './pages/Login';
import { Overview } from './pages/Overview';
import { AlmProvider } from './alm/state';
import { load } from './pages/routes';

const Blotter = lazy(() => load.blotter().then((m) => ({ default: m.Blotter })));
const Breaks = lazy(() => load.breaks().then((m) => ({ default: m.Breaks })));
const Ladder = lazy(() => load.liquidity().then((m) => ({ default: m.Ladder })));
const Alm = lazy(() => load.alm().then((m) => ({ default: m.Alm })));
const Risk = lazy(() => load.risk().then((m) => ({ default: m.Risk })));
const Ftp = lazy(() => load.ftp().then((m) => ({ default: m.Ftp })));
const Pmu = lazy(() => load.pmu().then((m) => ({ default: m.Pmu })));

function Gate() {
  const { current, canSeeAlm } = useSession();
  if (!current) return <Login />;
  return (
    <LiveProvider>
      <BrowserRouter basename="/app">
        <ToastProvider>
          <PaletteProvider>
            <Shell>
              <Suspense fallback={<PageSkeleton />}>
                <Routes>
                  <Route index element={<Overview />} />
                  <Route path="blotter" element={<Blotter />} />
                  <Route path="breaks" element={<Breaks />} />
                  <Route path="liquidity" element={<Ladder />} />
                  <Route path="risk/*" element={<Risk />} />
                  <Route path="ftp" element={<Ftp />} />
                  <Route path="pmu/*" element={<Pmu />} />
                  <Route
                    path="alm/:tool?"
                    element={
                      canSeeAlm ? (
                        <AlmProvider>
                          <Alm />
                        </AlmProvider>
                      ) : (
                        <Navigate to="/" replace />
                      )
                    }
                  />
                  <Route path="*" element={<Navigate to="/" replace />} />
                </Routes>
              </Suspense>
            </Shell>
          </PaletteProvider>
        </ToastProvider>
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
