const { test, expect } = require('playwright/test')

test.describe.configure({ mode: 'serial' })

const implementations = [
  {
    name: 'JavaScript',
    kind: 'js',
    origin: process.env.CATALOG_JS_ORIGIN || 'http://127.0.0.1:3027',
  },
  {
    name: 'native Rust',
    kind: 'rust',
    origin: process.env.CATALOG_RUST_ORIGIN || 'http://127.0.0.1:3039',
  },
]

const observations = new Map()

for (const implementation of implementations) {
  test(`${implementation.name} catalog streams, hydrates, and navigates`, async ({
    page,
    request,
  }) => {
    const listPath = `/catalog/${implementation.kind}`
    const errors = []
    const failedRequests = []
    const documents = []
    const flightRequests = []
    const segmentPrefetches = []

    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text())
    })
    page.on('pageerror', (error) => errors.push(error.message))
    page.on('requestfailed', (failedRequest) => {
      if (failedRequest.failure()?.errorText !== 'net::ERR_ABORTED') {
        failedRequests.push(
          `${failedRequest.url()}: ${failedRequest.failure()?.errorText}`
        )
      }
    })
    page.on('request', (browserRequest) => {
      if (browserRequest.resourceType() === 'document') {
        documents.push(browserRequest.url())
      }
      if (browserRequest.headers().rsc === '1') {
        flightRequests.push(browserRequest.url())
      }
      const segment = browserRequest.headers()['next-router-segment-prefetch']
      if (segment) segmentPrefetches.push(segment)
    })

    const streamedUrl = `${implementation.origin}${listPath}?categoryDelay=150&productDelay=500`
    await page.goto(streamedUrl, { waitUntil: 'commit' })
    await expect(
      page.locator('[data-loading-region="categories"]')
    ).toBeVisible()
    await expect(page.locator('[data-loading-region="products"]')).toBeVisible()
    await expect(page.locator('[data-product-id]')).toHaveCount(24)
    await expect(page.locator('[data-loading-region]')).toHaveCount(0)
    await expect(page.locator('[data-catalog]')).toHaveAttribute(
      'data-catalog',
      implementation.kind
    )
    await expect(page.locator('[data-catalog-controls-ready]')).toHaveAttribute(
      'data-catalog-controls-ready',
      'true'
    )

    const initialIds = await productIds(page)
    const firstId = initialIds[0]
    const detailPath = `${listPath}/product/${firstId}`
    const ssrDetail = await request.get(`${implementation.origin}${detailPath}`)
    expect(ssrDetail.status()).toBe(200)
    expect(await ssrDetail.text()).toContain(`data-product-detail="${firstId}"`)

    await page.evaluate(() => {
      window.__catalogParityMarker = 'list-to-detail'
    })
    const detailLink = page.locator(`a[href="${detailPath}"]`)
    await detailLink.hover()
    await detailLink.click()
    await expect(page).toHaveURL(new RegExp(`${escapeRegex(detailPath)}$`))
    await expect(page.locator('[data-product-detail]')).toHaveAttribute(
      'data-product-detail',
      firstId
    )
    expect(await page.evaluate(() => window.__catalogParityMarker)).toBe(
      'list-to-detail'
    )
    expect(documents).toHaveLength(1)

    await page.evaluate(() => {
      window.__catalogParityMarker = 'detail-to-list'
    })
    await page.locator(`a[href="${listPath}"]`).first().hover()
    await page.locator(`a[href="${listPath}"]`).first().click()
    await expect(page).toHaveURL(new RegExp(`${escapeRegex(listPath)}$`))
    await expect(page.locator('[data-product-id]')).toHaveCount(24)
    expect(await page.evaluate(() => window.__catalogParityMarker)).toBe(
      'detail-to-list'
    )
    expect(documents).toHaveLength(1)

    await page.locator('a[href="?category=fasteners"]').hover()
    await page.locator('a[href="?category=fasteners"]').click()
    await expect(page).toHaveURL(/category=fasteners/)
    await expect(page.locator('[data-product-id]')).toHaveCount(24)

    await page.locator('select[name="material"]').selectOption('Aluminum')
    await page.locator('select[name="sort"]').selectOption('price')
    await page.locator('.catalog-filters button').click()
    await expect(page).toHaveURL(/material=Aluminum.*sort=price/)
    await expect(page.locator('[data-product-id]')).toHaveCount(24)
    const filteredIds = await productIds(page)

    const slowRequest = page.waitForRequest((browserRequest) =>
      browserRequest.url().includes('productDelay=1000')
    )
    await page.evaluate(() => {
      const form = document.querySelector('.catalog-filters')
      form.querySelector('[name="productDelay"]').value = '1000'
      form.querySelector('[name="material"]').value = 'Zinc Steel'
      form.requestSubmit()
    })
    await slowRequest
    await page.evaluate(() => {
      const form = document.querySelector('.catalog-filters')
      form.querySelector('[name="productDelay"]').value = '50'
      form.querySelector('[name="material"]').value = 'Brass'
      form.requestSubmit()
    })
    await expect(page).toHaveURL(/productDelay=50.*material=Brass/)
    await expect(
      page.locator('.catalog-table tbody tr td:nth-child(3)', {
        hasText: 'Brass',
      })
    ).toHaveCount(24)

    expect(documents).toHaveLength(1)
    expect(flightRequests.length).toBeGreaterThanOrEqual(5)
    expect(errors).toEqual([])
    expect(failedRequests).toEqual([])

    observations.set(implementation.kind, {
      initialIds,
      filteredIds,
      segmentPrefetches,
    })
    if (implementation.kind === 'rust') {
      expect(observations.get('rust').initialIds).toEqual(
        observations.get('js').initialIds
      )
      expect(observations.get('rust').filteredIds).toEqual(
        observations.get('js').filteredIds
      )
      expect(segmentPrefetches.length).toBeGreaterThan(0)
    }
  })
}

async function productIds(page) {
  return page
    .locator('[data-product-id]')
    .evaluateAll((rows) =>
      rows.map((row) => row.getAttribute('data-product-id'))
    )
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
