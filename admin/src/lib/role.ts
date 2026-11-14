export interface Role {
  isAdmin: boolean
  username: string
}

export function currentRole(): Role {
  try {
    const user = JSON.parse(localStorage.getItem('user') || '{}')
    return { isAdmin: !!user.isAdmin, username: user.username ?? '' }
  } catch {
    return { isAdmin: false, username: '' }
  }
}
