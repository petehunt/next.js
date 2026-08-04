import { Suspense } from 'react'

export default function Page(props: { params: Promise<{ slug: string }> }) {
  return (
    <Suspense fallback={<p>Loading TypeScript child</p>}>
      <DynamicPage {...props} />
    </Suspense>
  )
}

async function DynamicPage({ params }: { params: Promise<{ slug: string }> }) {
  return <p>Dynamic TypeScript page: {(await params).slug}</p>
}
