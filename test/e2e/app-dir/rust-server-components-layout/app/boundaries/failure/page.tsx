import { Suspense } from 'react'
import { connection } from 'next/server'

export default function FailurePage() {
  return (
    <Suspense fallback={<p>Waiting to fail</p>}>
      <DynamicFailure />
    </Suspense>
  )
}

async function DynamicFailure(): Promise<never> {
  await connection()
  throw new Error('intentional fixture failure')
}
