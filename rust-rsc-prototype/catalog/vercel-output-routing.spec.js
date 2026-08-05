const { test, expect } = require('playwright/test')

const fallback = 'http://127.0.0.1:3133'
const nativeOnly = 'http://127.0.0.1:3134'

test('Build Output routes unconditional rewrites directly to Rust', async ({
  request,
}) => {
  for (const origin of [fallback, nativeOnly]) {
    const response = await request.get(`${origin}/native-rust-page`)
    expect(response.status()).toBe(200)
    expect(await response.text()).toContain('data-renderer="rust"')
  }
})

test('conditional rewrites retain their required Next fallback', async ({
  request,
}) => {
  const response = await request.get(`${fallback}/conditional-rust`, {
    headers: { 'x-use-rust': 'yes' },
  })
  expect(response.status()).toBe(200)
  expect(await response.text()).toContain('Rust layout around a Rust page')

  const refused = await request.get(`${nativeOnly}/conditional-rust`, {
    headers: { 'x-use-rust': 'yes' },
  })
  expect(refused.status()).toBe(404)
})
