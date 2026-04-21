import { useEffect, useState } from 'react'
import { BrowserRouter, Routes, Route, Navigate, useLocation } from 'react-router-dom'
import Layout from './layout/Layout'
import Dashboard from './pages/Dashboard'
import Providers from './pages/Providers'
import Config from './pages/Config'
import Metrics from './pages/Metrics'
import ApiKeys from './pages/ApiKeys'
import Login from './pages/Login'
import Setup from './pages/Setup'
import { api } from './api/client'
import { API_BASE_URL } from './api/client'

function PrivateRoute({ children }: { children: React.ReactNode }) {
  const location = useLocation()
  const [authenticated, setAuthenticated] = useState<boolean | null>(null)

  useEffect(() => {
    async function checkAuth() {
      const token = localStorage.getItem('token')
      if (!token) {
        setAuthenticated(false)
        return
      }
      
      try {
        const response = await fetch(`${API_BASE_URL}/api/auth/status`, {
          headers: { Authorization: `Bearer ${token}` }
        })
        const data = await response.json()
        setAuthenticated(data.authenticated)
      } catch {
        setAuthenticated(false)
      }
    }
    checkAuth()
  }, [])

  if (authenticated === null) {
    return <div className="min-h-screen flex items-center justify-center">Loading...</div>
  }

  return authenticated ? (
    <>{children}</>
  ) : (
    <Navigate to="/login" state={{ from: location }} replace />
  )
}

function SetupCheckRoute({ children }: { children: React.ReactNode }) {
  const [setupComplete, setSetupComplete] = useState<boolean | null>(null)

  useEffect(() => {
    api.checkSetupComplete().then(data => {
      setSetupComplete(data.setup_complete)
    }).catch(() => {
      setSetupComplete(false)
    })
  }, [])

  if (setupComplete === null) {
    return <div className="min-h-screen flex items-center justify-center">Loading...</div>
  }

  return setupComplete ? children : <Navigate to="/setup" replace />
}

function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/setup" element={<Setup />} />
        <Route path="/login" element={<Login />} />
        <Route path="/" element={
          <SetupCheckRoute>
            <PrivateRoute>
              <Layout />
            </PrivateRoute>
          </SetupCheckRoute>
        }>
          <Route index element={<Dashboard />} />
          <Route path="providers" element={<Providers />} />
          <Route path="config" element={<Config />} />
          <Route path="metrics" element={<Metrics />} />
          <Route path="api-keys" element={<ApiKeys />} />
        </Route>
      </Routes>
    </BrowserRouter>
  )
}

export default App
