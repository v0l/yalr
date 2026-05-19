import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"
import type { CurrencyAmount } from "../types"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function formatBalance(amount: number, currency: 'msats' | 'sats' | 'usd_micro'): string {
  switch (currency) {
    case 'msats':
      return `${Math.floor(amount / 1000)} sats`
    case 'sats':
      return `${amount.toLocaleString()} sats`
    case 'usd_micro':
      return `$${(amount / 1_000_000).toFixed(2)}`
    default:
      return `${amount}`
  }
}