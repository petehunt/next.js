export default function Page({ params, segment }) {
  return `<h2 id="slug">${params.slug}</h2><p id="segment">${segment}</p>`
}
