import { Link, Outlet, useLocation, useNavigate } from 'react-router-dom'
import { useTheme } from '../context/ThemeContext'
import { Button } from '@/components/ui/button'
import { MoonIcon, SunIcon, LogOutIcon } from 'lucide-react'

const navigation = [
  { name: 'Dashboard', path: '/' },
  { name: 'Providers', path: '/providers' },
  { name: 'Config', path: '/config' },
  { name: 'Metrics', path: '/metrics' },
  { name: 'Users', path: '/users' },
  { name: 'Chat', path: '/chat' },
]

export default function Layout() {
  const location = useLocation()
  const navigate = useNavigate()
  const { theme, toggleTheme } = useTheme()
  const user = JSON.parse(localStorage.getItem('user') || '{}')

  function handleLogout() {
    localStorage.removeItem('token')
    localStorage.removeItem('user')
    navigate('/login')
  }

  return (
    <div className="min-h-screen flex flex-col">
      <nav className="border-b bg-card">
        <div className="w-full px-4 sm:px-6 lg:px-8">
          <div className="flex justify-between h-14">
            <div className="flex">
              <div className="flex-shrink-0 flex items-center">
                <span className="text-lg font-bold">YALR Admin</span>
              </div>
              <div className="hidden sm:ml-6 sm:flex sm:gap-0">
                {navigation.map((item) => (
                  <Link
                    key={item.path}
                    to={item.path}
                    className={`inline-flex items-center px-3 py-1.5 text-sm font-medium border-b-2 transition-colors ${
                      location.pathname === item.path
                        ? 'border-primary text-foreground'
                        : 'border-transparent text-muted-foreground hover:text-foreground'
                    }`}
                  >
                    {item.name}
                  </Link>
                ))}
              </div>
            </div>
            <div className="flex items-center gap-3">
              <span className="text-sm text-muted-foreground">{user.username || 'User'}</span>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={toggleTheme}
                title={`Switch to ${theme === 'light' ? 'dark' : 'light'} mode`}
              >
                {theme === 'light' ? <MoonIcon /> : <SunIcon />}
              </Button>
              <Button variant="outline" size="sm" onClick={handleLogout}>
                <LogOutIcon />
                Logout
              </Button>
            </div>
          </div>
        </div>
      </nav>
      <main className="flex-1 w-full">
        <Outlet />
      </main>
    </div>
  )
}