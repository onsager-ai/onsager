import { useQuery } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useOptionalActiveWorkspace } from '@/lib/workspace';

export function useNodes() {
  // Wait for the AuthProvider's `/me` round-trip so the query doesn't
  // fire before the session cookie is confirmed (firing pre-login
  // floods the network tab with 401s). With auth always-on (#193) the
  // gate is just `!loading && user`.
  const { user, loading } = useAuth();
  const workspace = useOptionalActiveWorkspace();
  // CreateSessionSheet renders this hook from outside the scoped layout
  // (e.g. command palette before a workspace is picked). Skip the fetch
  // there — listing global nodes is meaningless once `/api/nodes`
  // requires `workspace_id` (#164).
  const enabled = !loading && !!user && !!workspace;

  return useQuery({
    queryKey: ['nodes', workspace?.id ?? ''],
    queryFn: () => api.getNodes(workspace!.id),
    refetchInterval: 5000,
    enabled,
  });
}
