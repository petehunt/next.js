export default async function BoundariesPage() {
  await new Promise((resolve) => setTimeout(resolve, 300))
  return <p>Boundary page resolved</p>
}
