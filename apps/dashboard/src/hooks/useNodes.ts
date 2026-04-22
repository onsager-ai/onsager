import { useQuery } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';

export function useNodes() {
  // When auth is enabled, `/api/nodes` returns 401 for anonymous requests;
  // firing the poll pre-login floods the console and the browser's network
  // tab with 401s. Gate on resolved auth state so the query only runs once
  // we know the user is logged in (or auth is disabled entirely).
  const { user, loading, authEnabled } = useAuth();
  const enabled = !loading && (!authEnabled || !!user);

  return useQuery({
    queryKey: ['nodes'],
    queryFn: api.getNodes,
    refetchInterval: 5000,
    enabled,
  });
}
