export default function DashboardLayout({ segment }) {
  return `<section id="dashboard" data-segment="${segment}">
    <!--next-slot:children-->
    <aside id="aside"><!-- next-slot:modal --></aside>
  </section>`
}
