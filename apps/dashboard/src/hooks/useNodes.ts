import { useQuery } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useOptionalActiveWorkspace } from '@/lib/workspace';

export function useNodes() {
  // When auth is enabled, `/api/nodes` returns 401 for anonymous requests;
  // firing the poll pre-login floods the console and the browser's network
  // tab with 401s. Gate on resolved auth state so the query only runs once
  // we know the user is logged in (or auth is disabled entirely).
  const { user, loading, authEnabled } = useAuth();
  const workspace = useOptionalActiveWorkspace();
  // CreateSessionSheet renders this hook from outside the scoped layout
  // (e.g. command palette before a workspace is picked). Skip the fetch
  // there — listing global nodes is meaningless once `/api/nodes`
  // requires `workspace_id` (#164).
  const enabled = !loading && (!authEnabled || !!user) && !!workspace;

  return useQuery({
    queryKey: ['nodes', workspace?.id ?? ''],
    queryFn: () => api.getNodes(workspace!.id),
    refetchInterval: 5000,
    enabled,
  });
}
