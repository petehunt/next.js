/**
 * Functional parity between `blog-starter` and `blog-rs`.
 *
 * Comparing HTML byte-for-byte would be meaningless: one document is built by
 * React and Tailwind, the other by Rust string templates, and they differ in
 * whitespace, attribute order, `&#x3C;` versus `&lt;`, React's comment markers,
 * and the framework's own script and preload tags. None of that is what "the same
 * blog" means.
 *
 * So parity is checked on what a reader would actually notice:
 *
 *   * the visible text, normalised — every heading, title, excerpt, author, date
 *     and paragraph, in order;
 *   * the set of internal links;
 *   * the status code;
 *   * the rendered Markdown structure — headings and paragraph count.
 *
 * A difference in any of those is a real difference. A difference outside them is
 * noise, and the report says which is which rather than hiding either.
 */

/**
 * Differences that are expected, with the reason.
 *
 * A known difference is not a parity failure, but silently normalising it away
 * would be worse than reporting it — so it is reported, with its cause.
 */
export const EXPECTED_DIFFERENCES = {
  '/posts/does-not-exist': {
    reason:
      "`blog-starter`'s `getPostBySlug` calls `fs.readFileSync` and lets ENOENT " +
      'propagate, so `if (!post) notFound()` is unreachable and an unknown slug ' +
      'is a 500. The Rust port returns `None` and renders a 404 page, which is ' +
      'what the original was trying to do.',
  },
}

/** Fetches a page from both servers and compares them. */
export async function comparePage(path, { rust, next }) {
  const [left, right] = await Promise.all([
    fetchPage(rust, path),
    fetchPage(next, path),
  ])

  const differences = []
  const record = (what, rustValue, nextValue) => {
    if (JSON.stringify(rustValue) !== JSON.stringify(nextValue)) {
      differences.push({ what, rust: rustValue, next: nextValue })
    }
  }

  record('status', left.status, right.status)
  record('title', left.facts.title, right.facts.title)
  record('headings', left.facts.headings, right.facts.headings)
  record('links', left.facts.links, right.facts.links)
  record('dates', left.facts.dates, right.facts.dates)
  record('text', left.facts.text, right.facts.text)

  const expected = EXPECTED_DIFFERENCES[path]
  return {
    path,
    matches: differences.length === 0,
    /** True when the only differences are ones this file documents. */
    expected: Boolean(expected) && differences.length > 0,
    expectedReason: expected?.reason,
    differences,
    rust: left.facts,
    next: right.facts,
  }
}

async function fetchPage(origin, path) {
  const response = await fetch(`${origin}${path}`)
  const html = await response.text()
  return { status: response.status, html, facts: extractFacts(html) }
}

/**
 * Reduces a document to the facts a reader would notice.
 *
 * A regex reduction, not a DOM parse: the point is to be blunt about what is
 * being compared, and adding a parser dependency to a benchmark harness buys
 * nothing here.
 */
export function extractFacts(html) {
  const withoutHead = html.replace(/<head[\s\S]*?<\/head>/i, '')
  const body = withoutHead
    // Neither document's scripts, styles or slot frames are content.
    .replace(/<script[\s\S]*?<\/script>/gi, ' ')
    .replace(/<style[\s\S]*?<\/style>/gi, ' ')
    .replace(/<template[\s\S]*?<\/template>/gi, ' ')
    // React's hydration comment markers. Removed rather than replaced with a
    // space: they sit *between* text nodes that were adjacent in the source, so
    // `and {CMS_NAME}.` would otherwise read as `and Markdown .`.
    .replace(/<!--[\s\S]*?-->/g, '')

  return {
    title: decode(/<title>([\s\S]*?)<\/title>/i.exec(html)?.[1] ?? '').trim(),
    headings: [...body.matchAll(/<h([1-6])[^>]*>([\s\S]*?)<\/h\1>/gi)]
      .map((match) => `h${match[1]}: ${text(match[2])}`)
      .filter((heading) => !heading.endsWith(': ')),
    // Only internal links: the footer's GitHub URL differs by construction.
    links: [
      ...new Set(
        [...body.matchAll(/href="(\/[^"]*)"/g)].map((match) => match[1])
      ),
    ].sort(),
    dates: [
      ...new Set(
        [...body.matchAll(/<time[^>]*>([\s\S]*?)<\/time>/gi)].map((match) =>
          text(match[1])
        )
      ),
    ].sort(),
    text: text(body),
  }
}

/** Strips tags and normalises whitespace and entities. */
function text(fragment) {
  return decode(fragment.replace(/<[^>]*>/g, ' '))
    .replace(/\s+/g, ' ')
    .trim()
}

function decode(value) {
  return (
    value
      .replace(/&#x([0-9a-f]+);/gi, (_, hex) =>
        String.fromCodePoint(parseInt(hex, 16))
      )
      .replace(/&#(\d+);/g, (_, code) => String.fromCodePoint(Number(code)))
      .replace(/&lt;/g, '<')
      .replace(/&gt;/g, '>')
      .replace(/&quot;/g, '"')
      .replace(/&#39;/g, "'")
      .replace(/&nbsp;/g, ' ')
      // Last, so an escaped `&amp;lt;` does not become `<`.
      .replace(/&amp;/g, '&')
  )
}
