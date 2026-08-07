/**
 * Context that is shared by every render in a given deployment, regardless of
 * which render protocol serves the route.
 *
 * This lives next to the protocol contract (rather than inside the React
 * renderer) because it is part of the protocol's input, not part of any one
 * renderer's implementation.
 */
export type AppSharedContext = {
  buildId: string
  deploymentId: string
  clientAssetToken: string
}
