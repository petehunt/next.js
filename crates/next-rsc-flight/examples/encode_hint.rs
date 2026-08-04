use std::io::{self, Write};

use next_rsc_flight::{FlightValue, HintCode, encode_hint_chunks, encode_root_value};

fn main() {
    let mut output = io::stdout().lock();
    for chunk in encode_hint_chunks(
        HintCode::DnsPrefetch,
        &FlightValue::from("https://assets.example"),
    )
    .unwrap()
    {
        output.write_all(&chunk).unwrap();
    }
    output
        .write_all(&encode_root_value(&FlightValue::from("hinted root")).unwrap())
        .unwrap();
}
