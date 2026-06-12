import { useQuery } from '@tanstack/react-query';
import { api } from '@/lib/api';

export function useSession(id: string) {
  return useQuery({
    queryKey: ['session', id],
    queryFn: () => api.getSession(id),
    refetchInterval: 3000,
  });
}
