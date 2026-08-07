export default async function Page({ params, segment }) {
  return `<h1 id="slug">${params.slug}</h1><p id="segment">${segment}</p>`
}
