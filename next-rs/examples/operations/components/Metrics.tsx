'use client'

export interface MetricsProps {
  stats: number[]
}

export default function Metrics({ stats }: MetricsProps) {
  return (
    <ul className="metrics">
      {stats.map((value, index) => (
        <li key={index}>{value}</li>
      ))}
    </ul>
  )
}
