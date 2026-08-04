use std::io::{self, Write};

use next_rsc::{Node, element};
use next_rsc_flight::{FlightValue, encode_root_value};

fn main() {
    let payload = FlightValue::object([
        ("string", FlightValue::from("$escaped")),
        ("number", FlightValue::Number(42.5)),
        (
            "bigint",
            FlightValue::BigInt("12345678901234567890".to_owned()),
        ),
        (
            "date",
            FlightValue::Date("2026-08-04T00:00:00.000Z".to_owned()),
        ),
        (
            "map",
            FlightValue::Map(vec![(FlightValue::from("key"), FlightValue::from("value"))]),
        ),
        (
            "set",
            FlightValue::Set(vec![FlightValue::from("a"), FlightValue::from("b")]),
        ),
        (
            "node",
            FlightValue::Node(element(
                "main",
                [
                    Node::text("same model"),
                    element("span", [Node::text("child")]),
                ],
            )),
        ),
    ]);
    io::stdout()
        .write_all(&encode_root_value(&payload).unwrap())
        .unwrap();
}
