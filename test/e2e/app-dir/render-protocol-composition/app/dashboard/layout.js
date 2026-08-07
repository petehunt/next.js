export default function DashboardLayout({ children, modal }) {
  return (
    <section id="dashboard">
      {children}
      <aside id="aside">{modal}</aside>
    </section>
  )
}
