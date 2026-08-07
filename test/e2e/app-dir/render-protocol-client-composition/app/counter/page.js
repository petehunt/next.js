import { Counter } from '../../components/counter'
import { Leave } from '../../components/leave'

export default function CounterPage() {
  return (
    <>
      <Counter name="page" />
      <Leave href="/" />
    </>
  )
}
