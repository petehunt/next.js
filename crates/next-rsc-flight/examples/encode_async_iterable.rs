use std::io::{self, Write};

use next_rsc_flight::{FlightValue, encode_root_value};

fn main() {
    let payload = FlightValue::object([
        (
            "iterable",
            FlightValue::AsyncIterable {
                iterator: false,
                values: vec![FlightValue::from("a"), FlightValue::from("b")],
                completion: None,
            },
        ),
        (
            "iterator",
            FlightValue::AsyncIterable {
                iterator: true,
                values: vec![FlightValue::from("x")],
                completion: Some(Box::new(FlightValue::from("finished"))),
            },
        ),
    ]);
    io::stdout()
        .write_all(&encode_root_value(&payload).unwrap())
        .unwrap();
}
