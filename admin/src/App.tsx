import { useEffect, useState } from 'react'
import { BrowserRouter, Routes, Route, Navigate, useLocation } from 'react-router-dom'
import Layout from './layout/Layout'
import Dashboard from './pages/Dashboard'
import Config from './pages/Config'
import Metrics from './pages/Metrics'
import Login from './pages/Login'
import Setup from './pages/Setup'
import Users from './pages/Users'
import UserDetail from './pages/UserDetail'
import Chat from './pages/Chat'
import RequestLog from './pages/RequestLog'
import RequestLogDetail from './pages/RequestLogDetail'
import Payments from './pages/Payments'
import Account from './pages/Account'
import ProviderDetail from './pages/ProviderDetail'
import Approvals from './pages/Approvals'
import Register from './pages/Register'
import Models from './pages/Models'
import ModelDetail from './pages/ModelDetail'
import { currentRole } from './lib/role'
import { api } from './api/client'
import { API_BASE_URL } from './api/client'

function LoadingScreen() {
  return (
    <div className="min-h-screen flex flex-col items-center justify-center bg-background gap-4">
      <div className="flex items-center justify-center w-12 h-12 bg-card border border-border">
        <svg viewBox="0 0 24 24" className="w-6 h-6" fill="none">
          <path d="M6 7l4 5-4 5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
          <path d="M12 17l4-10" stroke="currentColor" strokeWidth="2.5" strokeLinecap="square"/>
        </svg>
      </div>
      <span className="font-mono text-[13px] text-muted-foreground animate-blink">LOADING...</span>
    </div>
  )
}

function PrivateRoute({ children }: { children: React.ReactNode }) {
  const location = useLocation()
  const [authenticated, setAuthenticated] = useState<boolean | null>(null)

  useEffect(() => {
    async function checkAuth() {
      const token = localStorage.getItem('token')
      if (!token) { setAuthenticated(false); return }
      try {
        const response = await fetch(`${API_BASE_URL}/api/auth/status`, { headers: { Authorization: `Bearer ${token}` } })
        const data = await response.json()
        setAuthenticated(data.authenticated)
      } catch { setAuthenticated(false) }
    }
    checkAuth()
  }, [])

  if (authenticated === null) return <LoadingScreen />
  return authenticated ? <>{children}</> : <Navigate to="/login" state={{ from: location }} replace />
}

function SetupCheckRoute({ children }: { children: React.ReactNode }) {
  const [setupComplete, setSetupComplete] = useState<boolean | null>(null)

  useEffect(() => {
    api.checkSetupComplete().then(data => setSetupComplete(data.setup_complete)).catch(() => setSetupComplete(false))
  }, [])

  if (setupComplete === null) return <LoadingScreen />
  return setupComplete ? children : <Navigate to="/setup" replace />
}

function AdminRoute({ children }: { children: React.ReactNode }) {
  return currentRole().isAdmin ? <>{children}</> : <Navigate to="/" replace />
}

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/setup" element={<Setup />} />
        <Route path="/login" element={<Login />} />
        <Route path="/register" element={<Register />} />
        <Route path="/models" element={<Models />} />
        <Route path="/models/*" element={<ModelDetail />} />
        <Route path="/account/*" element={<Account />} />
        <Route path="/" element={
          <SetupCheckRoute>
            <PrivateRoute>
              <Layout />
            </PrivateRoute>
          </SetupCheckRoute>
        }>
          <Route index element={<Dashboard />} />
          <Route path="config" element={<Config />} />
          <Route path="metrics" element={<Metrics />} />
          <Route path="users" element={<AdminRoute><Users /></AdminRoute>} />
          <Route path="users/:id" element={<AdminRoute><UserDetail /></AdminRoute>} />
          <Route path="payments" element={<Payments />} />
          <Route path="chat" element={<Chat />} />
          <Route path="providers/:slug" element={<ProviderDetail />} />
          <Route path="approvals" element={<AdminRoute><Approvals /></AdminRoute>} />
          <Route path="request-log" element={<RequestLog />} />
          <Route path="request-log/:id" element={<RequestLogDetail />} />
        </Route>
      </Routes>
    </BrowserRouter>
  )
}
