//! A minimal server island, exercising the whole path: macro -> registry ->
//! N-API -> Node.

use maud::html;
use next_rs::{island, DeriveTsType};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DeriveTsType)]
#[serde(rename_all = "camelCase")]
pub struct ChartProps {
    pub rows: Vec<f64>,
    pub label_text: String,
}

#[derive(Serialize, DeriveTsType)]
#[serde(rename_all = "camelCase")]
pub struct LegendProps {
    pub max: f64,
    pub series: Vec<String>,
}

#[island]
#[slot(name = "legend", props = LegendProps)]
async fn chart(props: ChartProps) -> maud::Markup {
    // Pretend this is real work happening off the event loop.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let max = props.rows.iter().cloned().fold(f64::MIN, f64::max);

    html! {
        div class="chart" {
            span { (props.label_text) }
            ul { @for r in &props.rows { li { (r) } } }
            // The slot is written inside a maud template, which is the case the
            // in-band sentinel encoding exists for.
            (slot!("legend", LegendProps { max, series: vec!["revenue".into()] }))
        }
    }
}

// Plants the N-API entry points in this cdylib, where the linker keeps them.
next_napi::register!();
