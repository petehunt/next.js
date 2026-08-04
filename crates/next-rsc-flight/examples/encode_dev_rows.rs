use std::io::{self, Write};

use next_rsc::{Node, element};
use next_rsc_flight::{
    DevComponentInfo, DevStackFrame, FlightValue, encode_dev_component_chunks,
    encode_dev_time_origin, encode_dev_timing, encode_root_value,
};

fn main() {
    let mut output = io::stdout().lock();
    output.write_all(&encode_dev_time_origin(0.0)).unwrap();
    output.write_all(&encode_dev_timing(0, 0.0)).unwrap();
    for chunk in encode_dev_component_chunks(
        0,
        1,
        2,
        &DevComponentInfo {
            name: "RustPage".to_owned(),
            env: "Server".to_owned(),
            stack: vec![DevStackFrame {
                name: "RustPage".to_owned(),
                file: "app/page.rs".to_owned(),
                line: 3,
                column: 1,
            }],
        },
    ) {
        output.write_all(&chunk).unwrap();
    }
    output
        .write_all(
            &encode_root_value(&FlightValue::Node(element(
                "main",
                [Node::text("debug root")],
            )))
            .unwrap(),
        )
        .unwrap();
}
