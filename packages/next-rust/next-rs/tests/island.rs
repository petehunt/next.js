//! End-to-end exercise of the macro surface a user actually writes.

use std::collections::BTreeMap;

use next_rs::island;
use next_rs::ts::TsType;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, next_rs::DeriveTsType)]
#[serde(rename_all = "camelCase")]
struct ChartProps {
    rows: Vec<f64>,
    label_text: String,
    caption: Option<String>,
}

#[derive(Serialize, next_rs::DeriveTsType)]
#[serde(rename_all = "camelCase")]
struct LegendProps {
    max: f64,
    series: Vec<String>,
}

#[island]
#[slot(name = "legend", props = LegendProps)]
async fn chart(props: ChartProps) -> String {
    let max = props.rows.iter().cloned().fold(f64::MIN, f64::max);
    format!(
        "<div class=\"chart\"><span>{}</span>{}</div>",
        props.label_text,
        slot!("legend", LegendProps { max, series: vec!["a".into()] })
    )
}

#[tokio::test]
async fn island_renders_with_a_slot_carrying_rust_computed_props() {
    let props = serde_json::json!({ "rows": [1.0, 9.0, 3.0], "labelText": "Revenue" });
    let frag = next_rs::island::render("chart", props).await.expect("render");

    assert_eq!(frag.slot_names(), vec!["legend"]);
    let slot = frag.segments.iter().find_map(|s| match s {
        next_rs::Segment::Slot(d) => Some(d),
        _ => None,
    }).unwrap();
    // Rust computed this, not the author.
    assert_eq!(slot.props["max"], 9.0);
}

#[test]
fn manifest_exposes_declared_slots_and_prop_types() {
    let m = next_rs::island::manifest();
    let chart = m.iter().find(|e| e.id == "chart").expect("chart registered");
    assert_eq!(chart.props_ts, "ChartProps");
    assert_eq!(chart.slots.len(), 1);
    assert_eq!(chart.slots[0].name, "legend");
    assert_eq!(chart.slots[0].props_ts, "LegendProps");
}

#[test]
fn generated_dts_is_statically_declarable() {
    let mut decls = BTreeMap::new();
    ChartProps::ts_decls(&mut decls);
    LegendProps::ts_decls(&mut decls);
    let dts = next_rs::ts::render_decls(&decls);

    // snake_case -> camelCase, Option -> optional, Vec -> array.
    assert!(dts.contains("labelText: string"), "{dts}");
    assert!(dts.contains("caption?: string | null"), "{dts}");
    assert!(dts.contains("rows: number[]"), "{dts}");
    assert!(dts.contains("export type LegendProps"), "{dts}");
    assert!(!dts.contains("unknown"), "slot props must not degrade to unknown:\n{dts}");
}
