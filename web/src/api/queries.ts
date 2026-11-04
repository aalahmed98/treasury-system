// The page-level queries with the native client's cadences: feeds every 5 s, attention every 15 s,
// activity every 30 s, the ALM overview every 15 s, all passive so a dashboard never keeps an idle
// session alive. Deals, breaks and positions load once, are refetched by the live stream, and stay
// fresh for a minute so moving between pages never waits on the network.
import { QueryClient, useQuery } from '@tanstack/react-query';
import { endpoints } from './client';
import { useSession } from './session';

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, refetchOnWindowFocus: false, staleTime: 30_000 },
  },
});

const MINUTE = 60_000;

export const useFeeds = () => useQuery({ queryKey: ['feeds'], queryFn: endpoints.feeds, refetchInterval: 5_000, staleTime: 4_000 });
export const useAttention = () => useQuery({ queryKey: ['attention'], queryFn: endpoints.attention, refetchInterval: 15_000 });
export const useActivity = (limit = 20) => useQuery({ queryKey: ['activity', limit], queryFn: () => endpoints.activity(limit), refetchInterval: 30_000 });
export const useDeals = () => useQuery({ queryKey: ['deals'], queryFn: () => endpoints.deals(undefined, true), refetchInterval: MINUTE, staleTime: MINUTE });
export const useBreaks = (all = false) => useQuery({ queryKey: ['breaks', all], queryFn: () => endpoints.breaks(all, true), refetchInterval: MINUTE, staleTime: MINUTE });
export const usePositions = () => useQuery({ queryKey: ['positions'], queryFn: () => endpoints.positions(true), refetchInterval: MINUTE, staleTime: MINUTE });
export const useLadder = (ccy?: string) => useQuery({ queryKey: ['ladder', ccy ?? ''], queryFn: () => endpoints.ladder(ccy, true), refetchInterval: MINUTE, staleTime: MINUTE });

export function useAlmOverview() {
  const { canSeeAlm } = useSession();
  return useQuery({ queryKey: ['alm', 'overview'], queryFn: endpoints.almOverview, refetchInterval: 15_000, enabled: canSeeAlm });
}

export const useFxRisk = (enabled = true) => useQuery({ queryKey: ['risk', 'fx'], queryFn: endpoints.fxRisk, refetchInterval: MINUTE, staleTime: 20_000, enabled });
export const useFxHistory = (days = 30) => useQuery({ queryKey: ['risk', 'history', days], queryFn: () => endpoints.fxHistory(days), refetchInterval: 5 * MINUTE, staleTime: MINUTE });
export const useRiskLimits = () => useQuery({ queryKey: ['risk', 'limits'], queryFn: endpoints.riskLimits, staleTime: MINUTE });
export const useBreaches = (days = 30) => useQuery({ queryKey: ['risk', 'breaches', days], queryFn: () => endpoints.breaches(days), staleTime: MINUTE });
