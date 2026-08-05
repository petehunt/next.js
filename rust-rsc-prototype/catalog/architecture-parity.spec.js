const { test, expect } = require('playwright/test')

test.describe.configure({ mode: 'serial' })

const configurations = [
  {
    key: 'next-js',
    name: 'Conventional Next/JS',
    origin: process.env.CATALOG_NEXT_JS_ORIGIN || 'http://127.0.0.1:3027',
    path: '/catalog/js',
    catalog: 'js',
    streamsRegions: true,
  },
  {
    key: 'next-rust-wasm',
    name: 'Next + Rust/Wasm hybrid',
    origin: process.env.CATALOG_NEXT_WASM_ORIGIN || 'http://127.0.0.1:3027',
    path: '/catalog/rust',
    catalog: 'rust',
    streamsRegions: true,
  },
  {
    key: 'native-fallback',
    name: 'Native Rust with selective Next fallback',
    origin:
      process.env.CATALOG_NATIVE_FALLBACK_ORIGIN || 'http://127.0.0.1:3038',
    path: '/catalog/rust',
    catalog: 'rust',
    streamsRegions: true,
  },
  {
    key: 'native-only',
    name: 'Node-free native Rust',
    origin: process.env.CATALOG_NATIVE_ONLY_ORIGIN || 'http://127.0.0.1:3039',
    path: '/catalog/rust',
    catalog: 'rust',
    streamsRegions: true,
  },
]

const observations = new Map()

for (const configuration of configurations) {
  test(`${configuration.name}: feature and visual parity`, async ({
    page,
    request,
  }, testInfo) => {
    const errors = []
    const documents = []
    const flights = []
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text())
    })
    page.on('pageerror', (error) => errors.push(error.message))
    page.on('request', (browserRequest) => {
      if (browserRequest.resourceType() === 'document') {
        documents.push(browserRequest.url())
      }
      if (browserRequest.headers().rsc === '1')
        flights.push(browserRequest.url())
    })

    const delayed = `${configuration.origin}${configuration.path}?categoryDelay=150&productDelay=500`
    await page.goto(delayed, { waitUntil: 'commit' })
    if (configuration.streamsRegions) {
      await expect(
        page.locator('[data-loading-region="categories"]')
      ).toBeVisible()
      await expect(
        page.locator('[data-loading-region="products"]')
      ).toBeVisible()
    }
    await expect(page.locator('[data-product-id]')).toHaveCount(24)
    await expect(page.locator('[data-loading-region]')).toHaveCount(0)
    await expect(page.locator('[data-catalog-controls-ready]')).toHaveAttribute(
      'data-catalog-controls-ready',
      'true'
    )
    const initialIds = await productIds(page)

    await page.goto(`${configuration.origin}${configuration.path}`)
    await expect(page.locator('[data-product-id]')).toHaveCount(24)
    await expect(page.locator('[data-loading-region]')).toHaveCount(0)
    await expect(page.locator('[data-catalog-controls-ready]')).toHaveAttribute(
      'data-catalog-controls-ready',
      'true'
    )
    const listScreenshot = await page.screenshot({
      path: testInfo.outputPath(`${configuration.key}-list.png`),
      animations: 'disabled',
      caret: 'hide',
    })

    await page.evaluate(() => {
      window.__architectureParityMarker = 'preserved'
    })
    const detailHref = await page
      .locator('a[href*="/product/"]')
      .first()
      .getAttribute('href')
    await page.locator(`a[href="${detailHref}"]`).first().hover()
    await page.locator(`a[href="${detailHref}"]`).first().click()
    await expect(page.locator('[data-product-detail]')).toBeVisible()
    expect(await page.evaluate(() => window.__architectureParityMarker)).toBe(
      'preserved'
    )
    expect(documents).toHaveLength(2)
    const detailScreenshot = await page.screenshot({
      path: testInfo.outputPath(`${configuration.key}-detail.png`),
      animations: 'disabled',
      caret: 'hide',
    })
    await page.locator(`a[href="${configuration.path}"]`).first().click()
    await expect(page.locator('[data-product-id]')).toHaveCount(24)
    expect(documents).toHaveLength(2)

    await page.locator('select[name="material"]').selectOption('Brass')
    await page.locator('select[name="sort"]').selectOption('price')
    await page.locator('.catalog-filters button').click()
    await expect(page).toHaveURL(/material=Brass.*sort=price/)
    await expect(
      page.locator('.catalog-table tbody tr td:nth-child(3)', {
        hasText: 'Brass',
      })
    ).toHaveCount(24)
    const filteredIds = await productIds(page)

    const directDetail = await request.get(
      `${configuration.origin}${configuration.path}/product/${initialIds[0]}`
    )
    expect(directDetail.status()).toBe(200)
    expect(await directDetail.text()).toContain('data-product-detail')
    expect(flights.length).toBeGreaterThanOrEqual(3)
    expect(errors).toEqual([])

    observations.set(configuration.key, {
      initialIds,
      filteredIds,
      listScreenshot: listScreenshot.toString('base64'),
      detailScreenshot: detailScreenshot.toString('base64'),
    })
  })
}

test('all configurations render identical catalog pixels and data', async ({
  page,
}) => {
  const baseline = observations.get('next-js')
  for (const configuration of configurations.slice(1)) {
    const observation = observations.get(configuration.key)
    expect(observation.initialIds).toEqual(baseline.initialIds)
    expect(observation.filteredIds).toEqual(baseline.filteredIds)
    for (const [view, baselineImage, candidateImage] of [
      ['list', baseline.listScreenshot, observation.listScreenshot],
      ['detail', baseline.detailScreenshot, observation.detailScreenshot],
    ]) {
      const comparison = await comparePixels(
        page,
        baselineImage,
        candidateImage
      )
      expect(
        comparison.differingPixels / comparison.totalPixels,
        `${configuration.key} ${view} differing-pixel ratio`
      ).toBeLessThan(0.005)
      expect(
        comparison.maxChannelDelta,
        `${configuration.key} ${view} maximum channel delta`
      ).toBeLessThanOrEqual(80)
    }
  }
})

test('selective fallback proxies unsupported routes while native-only refuses them', async ({
  request,
}) => {
  const direct = await request.get(`${configurations[0].origin}/catalog/js`)
  const proxied = await request.get(`${configurations[2].origin}/catalog/js`)
  expect(proxied.status()).toBe(200)
  expect(direct.status()).toBe(200)
  expect(await direct.text()).toContain('data-catalog="js"')
  expect(await proxied.text()).toContain('data-catalog="js"')

  const nativeOnly = await request.get(`${configurations[3].origin}/catalog/js`)
  expect(nativeOnly.status()).toBe(404)
  expect(await nativeOnly.text()).toBe('Not found')
})

async function productIds(page) {
  return page
    .locator('[data-product-id]')
    .evaluateAll((rows) =>
      rows.map((row) => row.getAttribute('data-product-id'))
    )
}

async function comparePixels(page, left, right) {
  return page.evaluate(
    async ([leftBase64, rightBase64]) => {
      const load = (base64) =>
        new Promise((resolve, reject) => {
          const image = new Image()
          image.onload = () => resolve(image)
          image.onerror = reject
          image.src = `data:image/png;base64,${base64}`
        })
      const [leftImage, rightImage] = await Promise.all([
        load(leftBase64),
        load(rightBase64),
      ])
      if (
        leftImage.width !== rightImage.width ||
        leftImage.height !== rightImage.height
      ) {
        return { differingPixels: -1, maxChannelDelta: 255, totalPixels: 0 }
      }
      const canvas = document.createElement('canvas')
      canvas.width = leftImage.width
      canvas.height = leftImage.height
      const context = canvas.getContext('2d')
      context.drawImage(leftImage, 0, 0)
      const leftPixels = context.getImageData(
        0,
        0,
        canvas.width,
        canvas.height
      ).data
      context.clearRect(0, 0, canvas.width, canvas.height)
      context.drawImage(rightImage, 0, 0)
      const rightPixels = context.getImageData(
        0,
        0,
        canvas.width,
        canvas.height
      ).data
      let differingPixels = 0
      let maxChannelDelta = 0
      for (let index = 0; index < leftPixels.length; index += 4) {
        let different = false
        for (let channel = 0; channel < 4; channel++) {
          const delta = Math.abs(
            leftPixels[index + channel] - rightPixels[index + channel]
          )
          if (delta > 0) different = true
          maxChannelDelta = Math.max(maxChannelDelta, delta)
        }
        if (different) differingPixels++
      }
      return {
        differingPixels,
        maxChannelDelta,
        totalPixels: canvas.width * canvas.height,
      }
    },
    [left, right]
  )
}
