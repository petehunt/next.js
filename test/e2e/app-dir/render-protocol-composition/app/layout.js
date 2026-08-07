// No `renderProtocol` export: this route tree is React's, exactly as it was
// before protocols existed. Segments below opt out one subtree at a time.
export default function RootLayout({ children }) {
  return (
    <html>
      <body>
        <main id="root">{children}</main>
      </body>
    </html>
  )
}
