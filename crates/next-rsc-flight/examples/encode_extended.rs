use std::io::Write;

use next_rsc_flight::{BinaryKind, FlightValue, encode_root_value};

fn main() {
    let payload = FlightValue::object([
        (
            "bigint",
            FlightValue::BigInt("900719925474099312345".to_owned()),
        ),
        (
            "date",
            FlightValue::Date("2026-08-04T12:34:56.000Z".to_owned()),
        ),
        (
            "map",
            FlightValue::Map(vec![
                (FlightValue::from("alpha"), FlightValue::from("one")),
                (FlightValue::from("beta"), FlightValue::from("two")),
            ]),
        ),
        (
            "set",
            FlightValue::Set(vec![FlightValue::from("red"), FlightValue::from("blue")]),
        ),
        (
            "bytes",
            FlightValue::Uint8Array(vec![0, 1, 2, 127, 128, 255]),
        ),
        (
            "arrayBuffer",
            FlightValue::Binary(BinaryKind::ArrayBuffer, vec![1, 2, 3]),
        ),
        ("int8", FlightValue::Binary(BinaryKind::Int8, vec![255, 2])),
        (
            "clamped",
            FlightValue::Binary(BinaryKind::Uint8Clamped, vec![4, 5]),
        ),
        (
            "int16",
            FlightValue::Binary(BinaryKind::Int16, (-123_i16).to_le_bytes().to_vec()),
        ),
        (
            "uint16",
            FlightValue::Binary(BinaryKind::Uint16, 65000_u16.to_le_bytes().to_vec()),
        ),
        (
            "int32",
            FlightValue::Binary(BinaryKind::Int32, (-123456_i32).to_le_bytes().to_vec()),
        ),
        (
            "uint32",
            FlightValue::Binary(BinaryKind::Uint32, 4_000_000_000_u32.to_le_bytes().to_vec()),
        ),
        (
            "float32",
            FlightValue::Binary(BinaryKind::Float32, 1.5_f32.to_le_bytes().to_vec()),
        ),
        (
            "float64",
            FlightValue::Binary(BinaryKind::Float64, (-2.25_f64).to_le_bytes().to_vec()),
        ),
        (
            "bigint64",
            FlightValue::Binary(BinaryKind::BigInt64, (-9_i64).to_le_bytes().to_vec()),
        ),
        (
            "biguint64",
            FlightValue::Binary(BinaryKind::BigUint64, 10_u64.to_le_bytes().to_vec()),
        ),
        (
            "dataView",
            FlightValue::Binary(BinaryKind::DataView, vec![8, 9]),
        ),
        (
            "form",
            FlightValue::FormData(vec![("part".to_owned(), FlightValue::from("RC-1"))]),
        ),
        (
            "iterator",
            FlightValue::Iterator(vec![FlightValue::from("a"), FlightValue::from("b")]),
        ),
        (
            "blob",
            FlightValue::Blob {
                mime: "text/plain".to_owned(),
                chunks: vec![b"rust".to_vec(), b" flight".to_vec()],
            },
        ),
        (
            "stream",
            FlightValue::ReadableStream(vec![
                FlightValue::from("first"),
                FlightValue::from("second"),
            ]),
        ),
        (
            "byteStream",
            FlightValue::ByteStream(vec![b"rust".to_vec(), b"bytes".to_vec()]),
        ),
    ]);
    std::io::stdout()
        .write_all(&encode_root_value(&payload).unwrap())
        .unwrap();
}
