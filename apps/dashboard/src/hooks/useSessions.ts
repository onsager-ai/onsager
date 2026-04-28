import { useQuery } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { useActiveWorkspace } from '@/lib/workspace';

export function useSessions() {
  const workspace = useActiveWorkspace();
  return useQuery({
    queryKey: ['sessions', workspace.id],
    queryFn: () => api.getSessions(workspace.id),
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
