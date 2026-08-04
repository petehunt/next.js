use std::{convert::Infallible, hint::black_box, time::Instant};

use next_rsc::{Node, element};
use next_rsc_flight::{
    EncodeLimits, FlightSink, FlightTaskGraph, FlightValue, SinkStatus, encode_root_chunks,
};

const SAMPLES: usize = 9;
const ITERATIONS: usize = 20_000;

#[derive(Default)]
struct CountingSink(usize);

impl FlightSink for CountingSink {
    type Error = Infallible;

    fn write_chunk(&mut self, chunk: &[u8]) -> Result<SinkStatus, Self::Error> {
        self.0 += chunk.len();
        Ok(SinkStatus::Ready)
    }
}

fn sample(mut operation: impl FnMut()) -> Vec<f64> {
    (0..SAMPLES)
        .map(|_| {
            let started = Instant::now();
            for _ in 0..ITERATIONS {
                operation();
            }
            started.elapsed().as_nanos() as f64 / ITERATIONS as f64
        })
        .collect()
}

fn report(name: &str, mut values: Vec<f64>) {
    values.sort_by(f64::total_cmp);
    let median = values[values.len() / 2];
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    println!(
        "{{\"name\":\"{name}\",\"samples\":{},\"iterationsPerSample\":{ITERATIONS},\"medianNs\":\
         {median:.1},\"meanNs\":{mean:.1}}}",
        values.len()
    );
}

fn main() {
    report(
        "ir-construction",
        sample(|| {
            black_box(element(
                "article",
                [
                    element("h2", [Node::text("Stainless fastener")]),
                    element("p", [Node::text("$escaped \\\" catalog row")]),
                ],
            ));
        }),
    );

    let model = FlightValue::Node(element(
        "article",
        [
            element("h2", [Node::text("Stainless fastener")]),
            element("p", [Node::text("$escaped \\\" catalog row")]),
        ],
    ));
    report(
        "flight-framing-escaping",
        sample(|| {
            black_box(encode_root_chunks(black_box(&model)).unwrap());
        }),
    );

    report(
        "task-reserve-resolve-drain",
        sample(|| {
            let mut graph = FlightTaskGraph::new(EncodeLimits::default());
            let (task, pending) = graph.reserve_task().unwrap();
            graph.enqueue_root(&pending).unwrap();
            graph
                .resolve_task(task, &FlightValue::from("ready"))
                .unwrap();
            let mut sink = CountingSink::default();
            graph.drain_to_sink(&mut sink).unwrap();
            black_box(sink.0);
        }),
    );
}
