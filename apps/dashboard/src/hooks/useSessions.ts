import { useQuery } from '@tanstack/react-query';
import { api } from '@/lib/api';

export function useSessions() {
  return useQuery({
    queryKey: ['sessions'],
    queryFn: api.getSessions,
    refetchInterval: 5000,
  });
}

export function useSession(id: string) {
  return useQuery({
    queryKey: ['session', id],
    queryFn: () => api.getSession(id),
    refetchInterval: 3000,
  });
}
