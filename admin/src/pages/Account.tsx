import { useCallback, useState } from 'react'
import { Route, Routes } from 'react-router-dom'
import { accountKey } from '@/api/account'
import AccountChat from './account/AccountChat'
import AccountHome from './account/AccountHome'
import AccountModels from './account/AccountModels'
import AccountStart from './account/AccountStart'

export default function Account() {
  const [key, setKey] = useState<string | null>(() => accountKey.get())

  const signOut = useCallback(() => {
    accountKey.clear()
    setKey(null)
  }, [])

  if (!key) return <AccountStart onKey={setKey} />

  return (
    <Routes>
      <Route index element={<AccountHome accountKey={key} onSignOut={signOut} />} />
      <Route path="models" element={<AccountModels accountKey={key} onSignOut={signOut} />} />
      <Route path="chat" element={<AccountChat accountKey={key} onSignOut={signOut} />} />
    </Routes>
  )
}
