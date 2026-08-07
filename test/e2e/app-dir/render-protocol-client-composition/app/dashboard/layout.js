export default function DashboardLayout() {
  return `<div id="dashboard">
    <!--next-slot:children-->
    <aside id="aside"><!--next-slot:aside--></aside>
    <div id="panel"><!--next-slot:panel--></div>
  </div>`
}
