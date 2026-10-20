import { useCallback, useState } from 'react'
import { accountKey } from '@/api/account'
import AccountHome from './account/AccountHome'
import AccountStart from './account/AccountStart'

export default function Account() {
  const [key, setKey] = useState<string | null>(() => accountKey.get())

  const signOut = useCallback(() => {
    accountKey.clear()
    setKey(null)
  }, [])

  return key ? <AccountHome accountKey={key} onSignOut={signOut} /> : <AccountStart onKey={setKey} />
}
