import { BrowserRouter, Navigate, Route, Routes } from 'react-router-dom'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { AuthProvider, useAuth } from '@/lib/auth'
import { WorkspaceProvider } from '@/lib/workspace'
import { Shell } from '@/components/layout/Shell'
import { LoginPage } from '@/pages/LoginPage'
import { WorkflowsPage } from '@/pages/WorkflowsPage'
import { WorkflowDetailPage } from '@/pages/WorkflowDetailPage'
import { RunDetailPage } from '@/pages/RunDetailPage'
import { ArtifactDetailPage, ArtifactsPage } from '@/pages/ArtifactsPage'
import { ActivityPage } from '@/pages/ActivityPage'
import { CredentialsPage } from '@/pages/CredentialsPage'

const queryClient = new QueryClient()

function Authed({ children }: { children: React.ReactNode }) {
  const { user, loading } = useAuth()
  if (loading) {
    return (
      <div className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading...
      </div>
    )
  }
  if (!user) return <Navigate to="/login" replace />
  return <>{children}</>
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <BrowserRouter>
          <Routes>
            <Route path="/login" element={<LoginPage />} />
            <Route
              path="/*"
              element={
                <Authed>
                  <WorkspaceProvider>
                    <Shell>
                      <Routes>
                        <Route path="/" element={<Navigate to="/workflows" replace />} />
                        <Route path="/workflows" element={<WorkflowsPage />} />
                        <Route path="/workflows/:id" element={<WorkflowDetailPage />} />
                        <Route path="/runs/:id" element={<RunDetailPage />} />
                        <Route path="/artifacts" element={<ArtifactsPage />} />
                        <Route path="/artifacts/:id" element={<ArtifactDetailPage />} />
                        <Route path="/activity" element={<ActivityPage />} />
                        <Route path="/credentials" element={<CredentialsPage />} />
                      </Routes>
                    </Shell>
                  </WorkspaceProvider>
                </Authed>
              }
            />
          </Routes>
        </BrowserRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
}
