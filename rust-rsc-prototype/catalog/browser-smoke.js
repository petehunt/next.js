const { chromium } = require('playwright')

async function main() {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage()
  const consoleErrors = [],
    pageErrors = [],
    failed = [],
    stylesheets = []
  const rscResponses = []
  const segmentPrefetches = []
  const documents = []
  page.on(
    'request',
    (request) =>
      request.headers()['next-router-segment-prefetch'] &&
      segmentPrefetches.push(request.headers()['next-router-segment-prefetch'])
  )
  page.on(
    'console',
    (message) =>
      message.type() === 'error' && consoleErrors.push(message.text())
  )
  page.on('pageerror', (error) => pageErrors.push(error.message))
  page.on('requestfailed', (request) =>
    failed.push(`${request.url()}: ${request.failure()?.errorText}`)
  )
  page.on(
    'response',
    (response) =>
      response.url().endsWith('.css') && stylesheets.push(response.url())
  )
  page.on(
    'response',
    (response) =>
      /text\/x-component/.test(response.headers()['content-type'] || '') &&
      rscResponses.push(response.url())
  )
  page.on(
    'request',
    (request) =>
      request.resourceType() === 'document' && documents.push(request.url())
  )

  await page.goto(
    'http://127.0.0.1:3039/catalog/rust?categoryDelay=150&productDelay=500',
    { waitUntil: 'commit' }
  )
  await page.waitForSelector('[data-loading-region="categories"]')
  const fallbackVisible = await page
    .locator('[data-loading-region="products"]')
    .isVisible()
  await page.waitForSelector('[data-product-id]')
  await page.waitForFunction(() => document.readyState === 'complete')
  await page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve))
      )
  )
  const state = await page.evaluate(() => ({
    products: new Set(
      [...document.querySelectorAll('[data-product-id]')].map((node) =>
        node.getAttribute('data-product-id')
      )
    ).size,
    loadingRegions: document.querySelectorAll('[data-loading-region]').length,
    catalog: document
      .querySelector('[data-catalog]')
      ?.getAttribute('data-catalog'),
    bodyBackground: getComputedStyle(document.body).backgroundColor,
  }))
  const firstBefore = await page
    .locator('[data-product-id]')
    .first()
    .getAttribute('data-product-id')
  await page.evaluate(() => {
    window.__catalogNavigationMarker = 'survived'
  })
  await page.selectOption('select[name="material"]', 'Aluminum')
  await page.selectOption('select[name="sort"]', 'price')
  await page.click('.catalog-filters button')
  await page.waitForURL(/material=Aluminum.*sort=price/)
  let navigationUpdated = true
  try {
    await page.waitForFunction(
      (before) => {
        const product = document.querySelector('[data-product-id]')
        return product && product.getAttribute('data-product-id') !== before
      },
      firstBefore,
      { timeout: 5000 }
    )
  } catch {
    navigationUpdated = false
  }
  const navigation = await page.evaluate(() => ({
    marker: window.__catalogNavigationMarker,
    first: document
      .querySelector('[data-product-id]')
      ?.getAttribute('data-product-id'),
    busy: document.querySelector('.catalog-filters')?.getAttribute('aria-busy'),
    url: location.href,
  }))
  const slowRequest = page.waitForRequest((request) =>
    request.url().includes('productDelay=1000')
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
  await page.waitForURL(/productDelay=50.*material=Brass/)
  await page.waitForFunction(() => {
    const materials = [
      ...document.querySelectorAll('.catalog-table tbody tr td:nth-child(3)'),
    ]
    return (
      materials.length === 24 &&
      materials.every((node) => node.textContent === 'Brass')
    )
  })
  const superseded = await page.evaluate(() => ({
    marker: window.__catalogNavigationMarker,
    url: location.href,
    brassRows: [
      ...document.querySelectorAll('.catalog-table tbody tr td:nth-child(3)'),
    ].filter((node) => node.textContent === 'Brass').length,
  }))
  await page.evaluate(() => {
    window.__catalogNavigationMarker = 'survived-dashboard-navigation'
  })
  await page.hover('[data-native-navigation]')
  await page.click('[data-native-navigation]')
  await page.waitForURL(/\/dashboard$/)
  await page.waitForSelector('[data-slot="team"]')
  const dashboard = await page.evaluate(() => ({
    marker: window.__catalogNavigationMarker,
    heading: document.querySelector('h1')?.textContent,
    team: document.querySelector('[data-slot="team"]')?.textContent,
  }))
  await browser.close()
  const unexpectedFailures = failed.filter(
    (value) => !/ERR_ABORTED/.test(value)
  )
  console.log(
    JSON.stringify(
      {
        fallbackVisible,
        stylesheets,
        state,
        navigationUpdated,
        navigation,
        superseded,
        dashboard,
        documents,
        rscResponses,
        segmentPrefetches,
        consoleErrors,
        pageErrors,
        failed,
      },
      null,
      2
    )
  )
  if (
    !fallbackVisible ||
    state.products !== 24 ||
    state.loadingRegions !== 0 ||
    state.catalog !== 'rust' ||
    stylesheets.length === 0 ||
    !navigationUpdated ||
    navigation.marker !== 'survived' ||
    navigation.first === firstBefore ||
    superseded.marker !== 'survived' ||
    superseded.brassRows !== 24 ||
    dashboard.marker !== 'survived-dashboard-navigation' ||
    !dashboard.team ||
    documents.length !== 1 ||
    rscResponses.length < 2 ||
    segmentPrefetches.length === 0 ||
    consoleErrors.length ||
    pageErrors.length ||
    unexpectedFailures.length
  )
    process.exit(1)
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
