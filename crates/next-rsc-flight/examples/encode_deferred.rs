use next_rsc_flight::{FlightValue, encode_root_value};

fn main() {
    let payload = FlightValue::object([
        ("immediate", FlightValue::String("now".to_owned())),
        (
            "later",
            FlightValue::Deferred(Box::new(FlightValue::String("ready".to_owned()))),
        ),
    ]);
    print!(
        "{}",
        String::from_utf8(encode_root_value(&payload).unwrap()).unwrap()
    );
}
