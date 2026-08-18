// Next still owns this URL, so it keeps using Next rendering (spec §6).
export async function GET() {
  return Response.json({ plan: 'pro' })
}
