use std::convert::Infallible;

use next_rsc_flight::{EncodeLimits, FlightSink, FlightTaskGraph, FlightValue, SinkStatus};

#[derive(Default)]
struct StdoutSink;

impl FlightSink for StdoutSink {
    type Error = Infallible;

    fn write_chunk(&mut self, chunk: &[u8]) -> Result<SinkStatus, Self::Error> {
        print!("{}", String::from_utf8_lossy(chunk));
        Ok(SinkStatus::Ready)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = FlightTaskGraph::new(EncodeLimits::default());
    let (first_id, first) = graph.reserve_task()?;
    let (second_id, second) = graph.reserve_task()?;
    graph.enqueue_root(&FlightValue::object([("first", first), ("second", second)]))?;
    graph.drain_to_sink(&mut StdoutSink)?;
    graph.resolve_task(second_id, &FlightValue::from("second ready"))?;
    graph.resolve_task(first_id, &FlightValue::from("first ready"))?;
    graph.drain_to_sink(&mut StdoutSink)?;
    Ok(())
}
