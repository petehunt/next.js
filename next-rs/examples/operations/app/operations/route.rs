//! The complete example from spec §95: a Rust-owned HTML document containing
//! three React Client Component slots with three different call-site policies.

use next_rs::prelude::*;

use crate::react::{account, metrics, notifications};

pub async fn GET(req: Request) -> Result<Response> {
    let org_id = req.query().get_uint("org_id")?;
    let user_id = req
        .session()
        .ok_or_else(|| Error::unauthorized("sign in to view operations"))?
        .user_id()?;

    HTML::render(format!(
        r#"
        <!doctype html>

        <html>
          <head>
            <title>Operations</title>
          </head>

          <body>
            <header>
              {}
            </header>

            <main>
              <section>
                <h1>Metrics</h1>
                {}
              </section>

              <aside>
                {}
              </aside>
            </main>

            <footer>
              Rendered by Rust
            </footer>
          </body>
        </html>
        "#,
        // Browser mount only. The server request remains Rust-only: no React, no
        // Node, nothing to cold-start.
        account(user_id),
        // Initial markup is React SSR'd, and props then refresh directly from
        // Rust as JSON.
        metrics(org_id).ssr().swr(SWROptions {
            revalidate_on_focus: true,
            refresh_interval: Some(60_000),
            ..Default::default()
        }),
        // Browser mount only, but refresh on focus.
        notifications(user_id).swr(SWROptions {
            revalidate_on_focus: true,
            ..Default::default()
        }),
    ))
}
