import { expect, test } from 'bun:test'
import { mainClass, shellClass, sidebarClass, topBarClass } from './shellClasses'

test('main is full width below md and shrinkable', () => {
  const tokens = mainClass.split(' ')
  expect(tokens).toContain('min-w-0')
  expect(tokens).toContain('md:ml-56')
  expect(tokens).not.toContain('ml-56')
})

test('sidebar is off canvas on phones until opened, always shown from md', () => {
  expect(sidebarClass(false).split(' ')).toContain('-translate-x-full')
  expect(sidebarClass(true).split(' ')).not.toContain('-translate-x-full')
  expect(sidebarClass(false).split(' ')).toContain('md:translate-x-0')
})

test('top bar only shows below md', () => {
  expect(topBarClass.split(' ')).toContain('md:hidden')
  expect(shellClass).toContain('overflow-x-hidden')
})
