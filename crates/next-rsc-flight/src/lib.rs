//! Minimal React Flight encoder for the exact model subset represented by the
//! experimental `next-rsc` crate.

use std::{
    collections::{BTreeMap, VecDeque},
    fmt::Write,
    io,
};

use next_rsc::{ClientReference, Element, Node, PropValue, SlotId};

/// Exact React Flight implementation this encoder has been decoded against.
/// Manifest generation refuses to proceed when Next vendors another revision.
pub const SUPPORTED_REACT_FLIGHT_REVISION: &str = "19.3.0-canary-cbb046ab-20260731";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryKind {
    ArrayBuffer,
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
    BigInt64,
    BigUint64,
    DataView,
}

/// React Flight resource-hint opcode for the pinned vendored revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HintCode {
    DnsPrefetch,
    Preconnect,
    Preload,
    ModulePreload,
    Style,
    Script,
    ModuleScript,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsoleMethod {
    Log,
    Info,
    Warn,
    Error,
    Debug,
}

impl ConsoleMethod {
    fn name(self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Debug => "debug",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevComponentInfo {
    pub name: String,
    pub env: String,
    pub stack: Vec<DevStackFrame>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevStackFrame {
    pub name: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl HintCode {
    fn tag(self) -> char {
        match self {
            Self::DnsPrefetch => 'D',
            Self::Preconnect => 'C',
            Self::Preload => 'L',
            Self::ModulePreload => 'm',
            Self::Style => 'S',
            Self::Script => 'X',
            Self::ModuleScript => 'M',
        }
    }
}

impl BinaryKind {
    fn tag(self) -> u8 {
        match self {
            Self::ArrayBuffer => b'A',
            Self::Int8 => b'O',
            Self::Uint8 => b'o',
            Self::Uint8Clamped => b'U',
            Self::Int16 => b'S',
            Self::Uint16 => b's',
            Self::Int32 => b'L',
            Self::Uint32 => b'l',
            Self::Float32 => b'G',
            Self::Float64 => b'g',
            Self::BigInt64 => b'M',
            Self::BigUint64 => b'm',
            Self::DataView => b'V',
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FlightValue {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    String(String),
    BigInt(String),
    Date(String),
    Array(Vec<FlightValue>),
    Object(BTreeMap<String, FlightValue>),
    Map(Vec<(FlightValue, FlightValue)>),
    Set(Vec<FlightValue>),
    Uint8Array(Vec<u8>),
    Binary(BinaryKind, Vec<u8>),
    FormData(Vec<(String, FlightValue)>),
    Iterator(Vec<FlightValue>),
    Blob {
        mime: String,
        chunks: Vec<Vec<u8>>,
    },
    ReadableStream(Vec<FlightValue>),
    ByteStream(Vec<Vec<u8>>),
    AsyncIterable {
        /// `true` emits React's iterator (`x`) form; `false` emits iterable (`X`).
        iterator: bool,
        values: Vec<FlightValue>,
        completion: Option<Box<FlightValue>>,
    },
    Node(Node),
    Deferred(Box<FlightValue>),
    /// Reference to a request task whose resolution row will be emitted later.
    Pending(u32),
}

impl FlightValue {
    pub fn object(entries: impl IntoIterator<Item = (impl Into<String>, FlightValue)>) -> Self {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }
}

impl From<&str> for FlightValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for FlightValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<bool> for FlightValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<Node> for FlightValue {
    fn from(value: Node) -> Self {
        Self::Node(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlightEncodeError {
    UnresolvedSlot(SlotId),
    LimitExceeded(&'static str),
    InvalidTask(&'static str),
}

impl std::fmt::Display for FlightEncodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnresolvedSlot(SlotId(id)) => {
                write!(formatter, "cannot encode unresolved React slot {id}")
            }
            Self::LimitExceeded(limit) => {
                write!(formatter, "Flight encode limit exceeded: {limit}")
            }
            Self::InvalidTask(reason) => write!(formatter, "invalid Flight task: {reason}"),
        }
    }
}

impl std::error::Error for FlightEncodeError {}

/// Encodes one completed root model as Flight row zero.
///
/// Convenience encoder for a completed model. Incremental native routes use
/// [`FlightTaskGraph`] to schedule later work and drain bounded chunks.
pub fn encode_root(node: &Node) -> Result<Vec<u8>, FlightEncodeError> {
    encode_root_value(&FlightValue::Node(node.clone()))
}

pub fn encode_root_value(value: &FlightValue) -> Result<Vec<u8>, FlightEncodeError> {
    let chunks = encode_root_chunks(value)?;
    let capacity = chunks.iter().map(Vec::len).sum();
    let mut output = Vec::with_capacity(capacity);
    for chunk in chunks {
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

/// Encodes a React Flight `:H` resource-hint row. Callers are responsible for
/// request-level deduplication using the same URL/options key they use for
/// host resource registration.
pub fn encode_hint_chunks(
    code: HintCode,
    model: &FlightValue,
) -> Result<Vec<Vec<u8>>, FlightEncodeError> {
    let limits = EncodeLimits::default();
    validate_value(model, limits)?;
    let mut encoder = Encoder::default();
    let mut row = format!(":H{}", code.tag());
    encoder.push_value(&mut row, model)?;
    row.push('\n');

    let mut chunks = Vec::with_capacity(
        encoder.imports.len() + encoder.binary_rows.len() + encoder.rows.len() + 1,
    );
    chunks.extend(encoder.imports.into_iter().map(String::into_bytes));
    chunks.extend(encoder.binary_rows);
    chunks.extend(encoder.rows.into_iter().map(String::into_bytes));
    chunks.push(row.into_bytes());
    Ok(chunks)
}

pub fn encode_dev_time_origin(time_origin_ms: f64) -> Vec<u8> {
    format!(":N{time_origin_ms}\n").into_bytes()
}

pub fn encode_dev_timing(chunk_id: u32, elapsed_ms: f64) -> Vec<u8> {
    format!("{chunk_id:x}:D{{\"time\":{elapsed_ms}}}\n").into_bytes()
}

fn push_dev_stack(output: &mut String, stack: &[DevStackFrame]) {
    for (index, frame) in stack.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push('[');
        push_json_string(output, &frame.name);
        output.push(',');
        push_json_string(output, &frame.file);
        write!(
            output,
            ",{},{},{},{},false]",
            frame.line, frame.column, frame.line, frame.column
        )
        .unwrap();
    }
}

pub fn encode_dev_component_chunks(
    target_chunk_id: u32,
    info_chunk_id: u32,
    stack_chunk_id: u32,
    info: &DevComponentInfo,
) -> Vec<Vec<u8>> {
    let mut info_row = format!("{info_chunk_id:x}:{{\"name\":");
    push_json_string(&mut info_row, &info.name);
    info_row.push_str(",\"key\":null,\"env\":");
    push_json_string(&mut info_row, &info.env);
    info_row.push_str(",\"stack\":[");
    push_dev_stack(&mut info_row, &info.stack);
    info_row.push_str("],\"props\":{}}\n");

    let mut stack_row = format!("{stack_chunk_id:x}:[");
    push_dev_stack(&mut stack_row, &info.stack);
    stack_row.push_str("]\n");
    vec![
        info_row.into_bytes(),
        stack_row.into_bytes(),
        format!("{target_chunk_id:x}:D\"${info_chunk_id:x}\"\n").into_bytes(),
    ]
}

pub fn encode_dev_console_chunks(
    method: ConsoleMethod,
    stack: &[DevStackFrame],
    env: &str,
    args: &[FlightValue],
) -> Result<Vec<Vec<u8>>, FlightEncodeError> {
    let model = FlightValue::Array(
        [
            FlightValue::from(method.name()),
            FlightValue::Array(
                stack
                    .iter()
                    .map(|frame| {
                        FlightValue::Array(vec![
                            FlightValue::from(frame.name.clone()),
                            FlightValue::from(frame.file.clone()),
                            FlightValue::Number(frame.line.into()),
                            FlightValue::Number(frame.column.into()),
                            FlightValue::Number(frame.line.into()),
                            FlightValue::Number(frame.column.into()),
                            FlightValue::Bool(false),
                        ])
                    })
                    .collect(),
            ),
            FlightValue::Null,
            FlightValue::from(env),
        ]
        .into_iter()
        .chain(args.iter().cloned())
        .collect(),
    );
    validate_value(&model, EncodeLimits::default())?;
    let mut encoder = Encoder::default();
    let mut row = String::from(":W");
    encoder.push_value(&mut row, &model)?;
    row.push('\n');
    let mut chunks = Vec::new();
    chunks.extend(encoder.imports.into_iter().map(String::into_bytes));
    chunks.extend(encoder.binary_rows);
    chunks.extend(encoder.rows.into_iter().map(String::into_bytes));
    chunks.push(row.into_bytes());
    Ok(chunks)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodeLimits {
    pub max_depth: usize,
    pub max_items: usize,
    pub max_string_bytes: usize,
    pub max_chunks: usize,
    pub max_buffered_bytes: usize,
}

impl Default for EncodeLimits {
    fn default() -> Self {
        Self {
            max_depth: 128,
            max_items: 100_000,
            max_string_bytes: 1024 * 1024,
            max_chunks: 4096,
            max_buffered_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Returns independently writable protocol chunks. This keeps import and large
/// text rows separate from the root model so an HTTP sink can honor backpressure.
pub fn encode_root_chunks(value: &FlightValue) -> Result<Vec<Vec<u8>>, FlightEncodeError> {
    encode_root_chunks_with_limits(value, EncodeLimits::default())
}

pub fn encode_root_chunks_with_limits(
    value: &FlightValue,
    limits: EncodeLimits,
) -> Result<Vec<Vec<u8>>, FlightEncodeError> {
    validate_value(value, limits)?;
    let mut encoder = Encoder::default();
    let mut root = String::from("0:");
    encoder.push_value(&mut root, value)?;
    root.push('\n');

    let mut chunks = Vec::with_capacity(
        encoder.imports.len() + encoder.binary_rows.len() + encoder.rows.len() + 1,
    );
    chunks.extend(encoder.imports.into_iter().map(String::into_bytes));
    chunks.extend(encoder.binary_rows);
    chunks.extend(encoder.rows.into_iter().map(String::into_bytes));
    chunks.push(root.into_bytes());
    chunks.extend(encoder.deferred_rows.into_iter().map(String::into_bytes));
    if chunks.len() > limits.max_chunks {
        return Err(FlightEncodeError::LimitExceeded("chunk count"));
    }
    let buffered_bytes = chunks.iter().try_fold(0usize, |total, chunk| {
        total
            .checked_add(chunk.len())
            .filter(|value| *value <= limits.max_buffered_bytes)
            .ok_or(FlightEncodeError::LimitExceeded("buffered bytes"))
    })?;
    debug_assert!(buffered_bytes <= limits.max_buffered_bytes);
    Ok(chunks)
}

enum Work<'a> {
    Value(&'a FlightValue, usize),
    Node(&'a Node, usize),
}

fn validate_value(value: &FlightValue, limits: EncodeLimits) -> Result<(), FlightEncodeError> {
    let mut stack = vec![Work::Value(value, 0)];
    let mut items = 0usize;
    let mut source_bytes = 0usize;
    while let Some(work) = stack.pop() {
        items += 1;
        if items > limits.max_items {
            return Err(FlightEncodeError::LimitExceeded("item count"));
        }
        let depth = match work {
            Work::Value(value, depth) => {
                match value {
                    FlightValue::String(value)
                    | FlightValue::BigInt(value)
                    | FlightValue::Date(value) => {
                        add_source_string(value, &mut source_bytes, limits)?
                    }
                    FlightValue::Array(values) => {
                        stack.extend(values.iter().map(|value| Work::Value(value, depth + 1)))
                    }
                    FlightValue::Object(values) => {
                        for (name, value) in values {
                            add_source_string(name, &mut source_bytes, limits)?;
                            stack.push(Work::Value(value, depth + 1));
                        }
                    }
                    FlightValue::Map(entries) => {
                        for (key, value) in entries {
                            stack.push(Work::Value(key, depth + 1));
                            stack.push(Work::Value(value, depth + 1));
                        }
                    }
                    FlightValue::Set(values) => {
                        stack.extend(values.iter().map(|value| Work::Value(value, depth + 1)))
                    }
                    FlightValue::Uint8Array(bytes) | FlightValue::Binary(_, bytes) => {
                        source_bytes = source_bytes
                            .checked_add(bytes.len())
                            .filter(|value| *value <= limits.max_buffered_bytes)
                            .ok_or(FlightEncodeError::LimitExceeded("source bytes"))?;
                    }
                    FlightValue::FormData(entries) => {
                        for (name, value) in entries {
                            add_source_string(name, &mut source_bytes, limits)?;
                            stack.push(Work::Value(value, depth + 1));
                        }
                    }
                    FlightValue::Iterator(values) => {
                        stack.extend(values.iter().map(|value| Work::Value(value, depth + 1)))
                    }
                    FlightValue::ReadableStream(values) => {
                        stack.extend(values.iter().map(|value| Work::Value(value, depth + 1)))
                    }
                    FlightValue::AsyncIterable {
                        values, completion, ..
                    } => {
                        stack.extend(values.iter().map(|value| Work::Value(value, depth + 1)));
                        if let Some(completion) = completion {
                            stack.push(Work::Value(completion, depth + 1));
                        }
                    }
                    FlightValue::ByteStream(chunks) => {
                        for bytes in chunks {
                            source_bytes = source_bytes
                                .checked_add(bytes.len())
                                .filter(|value| *value <= limits.max_buffered_bytes)
                                .ok_or(FlightEncodeError::LimitExceeded("source bytes"))?;
                        }
                    }
                    FlightValue::Blob { mime, chunks } => {
                        add_source_string(mime, &mut source_bytes, limits)?;
                        for bytes in chunks {
                            source_bytes = source_bytes
                                .checked_add(bytes.len())
                                .filter(|value| *value <= limits.max_buffered_bytes)
                                .ok_or(FlightEncodeError::LimitExceeded("source bytes"))?;
                        }
                    }
                    FlightValue::Node(node) => stack.push(Work::Node(node, depth + 1)),
                    FlightValue::Deferred(value) => stack.push(Work::Value(value, depth + 1)),
                    FlightValue::Pending(_) => {}
                    _ => {}
                }
                depth
            }
            Work::Node(node, depth) => {
                match node {
                    Node::Text(value) => add_source_string(value, &mut source_bytes, limits)?,
                    Node::Element(element) => {
                        add_source_string(&element.tag, &mut source_bytes, limits)?;
                        validate_props(&element.props, &mut source_bytes, limits)?;
                        stack.extend(element.props.values().filter_map(|value| match value {
                            PropValue::Node(node) => Some(Work::Node(node, depth + 1)),
                            _ => None,
                        }));
                        stack.extend(
                            element
                                .children
                                .iter()
                                .map(|node| Work::Node(node, depth + 1)),
                        );
                    }
                    Node::ClientReference(reference) => {
                        add_source_string(&reference.module_id, &mut source_bytes, limits)?;
                        add_source_string(&reference.export_name, &mut source_bytes, limits)?;
                        for chunk in &reference.chunks {
                            add_source_string(chunk, &mut source_bytes, limits)?;
                        }
                        validate_props(&reference.props, &mut source_bytes, limits)?;
                        stack.extend(reference.props.values().filter_map(|value| match value {
                            PropValue::Node(node) => Some(Work::Node(node, depth + 1)),
                            _ => None,
                        }));
                        stack.extend(
                            reference
                                .children
                                .iter()
                                .map(|node| Work::Node(node, depth + 1)),
                        );
                    }
                    Node::Fragment(children) => {
                        stack.extend(children.iter().map(|node| Work::Node(node, depth + 1)))
                    }
                    Node::ReactFragment { key, children } => {
                        if let Some(key) = key {
                            add_source_string(key, &mut source_bytes, limits)?;
                        }
                        stack.extend(children.iter().map(|node| Work::Node(node, depth + 1)))
                    }
                    Node::Suspense { fallback, content } => {
                        stack.push(Work::Node(fallback, depth + 1));
                        stack.push(Work::Node(content, depth + 1));
                    }
                    Node::Null | Node::Slot(_) => {}
                }
                depth
            }
        };
        if depth > limits.max_depth {
            return Err(FlightEncodeError::LimitExceeded("nesting depth"));
        }
    }
    Ok(())
}

fn validate_props(
    props: &BTreeMap<String, PropValue>,
    source_bytes: &mut usize,
    limits: EncodeLimits,
) -> Result<(), FlightEncodeError> {
    for (name, value) in props {
        add_source_string(name, source_bytes, limits)?;
        match value {
            PropValue::String(value) => add_source_string(value, source_bytes, limits)?,
            PropValue::Strings(values) => {
                for value in values {
                    add_source_string(value, source_bytes, limits)?;
                }
            }
            PropValue::Style(values) => {
                for (name, value) in values {
                    add_source_string(name, source_bytes, limits)?;
                    add_source_string(value, source_bytes, limits)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn add_source_string(
    value: &str,
    total: &mut usize,
    limits: EncodeLimits,
) -> Result<(), FlightEncodeError> {
    if value.len() > limits.max_string_bytes {
        return Err(FlightEncodeError::LimitExceeded("string bytes"));
    }
    *total = total
        .checked_add(value.len())
        .filter(|value| *value <= limits.max_buffered_bytes)
        .ok_or(FlightEncodeError::LimitExceeded("source bytes"))?;
    Ok(())
}

pub fn write_root_value(
    sink: &mut impl io::Write,
    value: &FlightValue,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for chunk in encode_root_chunks(value)? {
        sink.write_all(&chunk)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlightCancelled;

impl std::fmt::Display for FlightCancelled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Flight stream cancelled")
    }
}

impl std::error::Error for FlightCancelled {}

pub fn write_root_value_with_cancel(
    sink: &mut impl io::Write,
    value: &FlightValue,
    mut cancelled: impl FnMut() -> bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for chunk in encode_root_chunks(value)? {
        if cancelled() {
            return Err(Box::new(FlightCancelled));
        }
        sink.write_all(&chunk)?;
    }
    Ok(())
}

/// Encodes the rows that resolve a previously emitted [`FlightValue::Pending`].
/// IDs allocated by the resolved value start after `task_id`, preventing
/// collisions with the root stream that reserved that task.
pub fn encode_task_resolution_chunks(
    task_id: u32,
    value: &FlightValue,
) -> Result<Vec<Vec<u8>>, FlightEncodeError> {
    let limits = EncodeLimits::default();
    validate_value(value, limits)?;
    let mut encoder = Encoder {
        next_chunk_id: task_id + 1,
        ..Encoder::default()
    };
    let mut resolution = format!("{task_id:x}:");
    encoder.push_value(&mut resolution, value)?;
    resolution.push('\n');
    let mut chunks = Vec::new();
    chunks.extend(encoder.imports.into_iter().map(String::into_bytes));
    chunks.extend(encoder.binary_rows);
    chunks.extend(encoder.rows.into_iter().map(String::into_bytes));
    chunks.push(resolution.into_bytes());
    chunks.extend(encoder.deferred_rows.into_iter().map(String::into_bytes));
    Ok(chunks)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskState {
    Pending,
    Resolved,
    Errored,
}

/// Result of offering a single complete Flight chunk to a non-blocking sink.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SinkStatus {
    Ready,
    Backpressure,
}

/// A response sink must either accept the whole chunk or report backpressure.
/// This makes retrying after a socket becomes writable unambiguous.
pub trait FlightSink {
    type Error;

    fn write_chunk(&mut self, chunk: &[u8]) -> Result<SinkStatus, Self::Error>;
}

/// Incremental, request-owned Flight task graph.
///
/// Producers reserve task IDs before constructing the root model, then resolve
/// those tasks in whatever order their asynchronous work completes. The graph
/// owns protocol IDs and the bounded output queue for the lifetime of one
/// request; no request state is global.
pub struct FlightTaskGraph {
    limits: EncodeLimits,
    next_chunk_id: u32,
    tasks: BTreeMap<u32, TaskState>,
    queued: VecDeque<Vec<u8>>,
    queued_bytes: usize,
    emitted_chunks: usize,
    root_enqueued: bool,
    cancelled: bool,
}

impl FlightTaskGraph {
    pub fn new(limits: EncodeLimits) -> Self {
        Self {
            limits,
            next_chunk_id: 1,
            tasks: BTreeMap::new(),
            queued: VecDeque::new(),
            queued_bytes: 0,
            emitted_chunks: 0,
            root_enqueued: false,
            cancelled: false,
        }
    }

    pub fn reserve_task(&mut self) -> Result<(u32, FlightValue), FlightEncodeError> {
        self.ensure_active()?;
        let id = self.allocate_id()?;
        self.tasks.insert(id, TaskState::Pending);
        Ok((id, FlightValue::Pending(id)))
    }

    pub fn enqueue_root(&mut self, value: &FlightValue) -> Result<(), FlightEncodeError> {
        self.ensure_active()?;
        if self.root_enqueued {
            return Err(FlightEncodeError::InvalidTask("root already enqueued"));
        }
        let (chunks, next_id) =
            encode_numbered_value_chunks(0, self.next_chunk_id, value, self.limits)?;
        self.enqueue_chunks(chunks)?;
        self.next_chunk_id = next_id;
        self.root_enqueued = true;
        Ok(())
    }

    pub fn resolve_task(
        &mut self,
        task_id: u32,
        value: &FlightValue,
    ) -> Result<(), FlightEncodeError> {
        self.ensure_pending(task_id)?;
        let (chunks, next_id) =
            encode_numbered_value_chunks(task_id, self.next_chunk_id, value, self.limits)?;
        self.enqueue_chunks(chunks)?;
        self.next_chunk_id = next_id;
        self.tasks.insert(task_id, TaskState::Resolved);
        Ok(())
    }

    pub fn error_task(&mut self, task_id: u32, digest: &str) -> Result<(), FlightEncodeError> {
        self.ensure_pending(task_id)?;
        self.enqueue_chunks(vec![encode_error_for_chunk(task_id, digest)])?;
        self.tasks.insert(task_id, TaskState::Errored);
        Ok(())
    }

    /// Drains until empty or until the sink reports backpressure. A rejected
    /// chunk remains at the front and is offered exactly once on the next call.
    pub fn drain_to_sink<S: FlightSink>(&mut self, sink: &mut S) -> Result<SinkStatus, S::Error> {
        while let Some(chunk) = self.queued.front() {
            if sink.write_chunk(chunk)? == SinkStatus::Backpressure {
                return Ok(SinkStatus::Backpressure);
            }
            let chunk = self.queued.pop_front().expect("front chunk exists");
            self.queued_bytes -= chunk.len();
            self.emitted_chunks += 1;
        }
        Ok(SinkStatus::Ready)
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.queued.clear();
        self.queued_bytes = 0;
    }

    pub fn queued_bytes(&self) -> usize {
        self.queued_bytes
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn ensure_active(&self) -> Result<(), FlightEncodeError> {
        if self.cancelled {
            Err(FlightEncodeError::InvalidTask("request cancelled"))
        } else {
            Ok(())
        }
    }

    fn ensure_pending(&self, task_id: u32) -> Result<(), FlightEncodeError> {
        self.ensure_active()?;
        match self.tasks.get(&task_id) {
            Some(TaskState::Pending) => Ok(()),
            Some(_) => Err(FlightEncodeError::InvalidTask("task already completed")),
            None => Err(FlightEncodeError::InvalidTask("unknown task")),
        }
    }

    fn allocate_id(&mut self) -> Result<u32, FlightEncodeError> {
        let id = self.next_chunk_id;
        self.next_chunk_id = self
            .next_chunk_id
            .checked_add(1)
            .ok_or(FlightEncodeError::LimitExceeded("chunk id"))?;
        Ok(id)
    }

    fn enqueue_chunks(&mut self, chunks: Vec<Vec<u8>>) -> Result<(), FlightEncodeError> {
        let new_count = self
            .emitted_chunks
            .checked_add(self.queued.len())
            .and_then(|count| count.checked_add(chunks.len()))
            .ok_or(FlightEncodeError::LimitExceeded("chunk count"))?;
        if new_count > self.limits.max_chunks {
            return Err(FlightEncodeError::LimitExceeded("chunk count"));
        }
        let added_bytes = chunks.iter().try_fold(0usize, |total, chunk| {
            total
                .checked_add(chunk.len())
                .ok_or(FlightEncodeError::LimitExceeded("buffered bytes"))
        })?;
        let new_bytes = self
            .queued_bytes
            .checked_add(added_bytes)
            .filter(|bytes| *bytes <= self.limits.max_buffered_bytes)
            .ok_or(FlightEncodeError::LimitExceeded("buffered bytes"))?;
        self.queued.extend(chunks);
        self.queued_bytes = new_bytes;
        Ok(())
    }
}

fn encode_numbered_value_chunks(
    row_id: u32,
    next_chunk_id: u32,
    value: &FlightValue,
    limits: EncodeLimits,
) -> Result<(Vec<Vec<u8>>, u32), FlightEncodeError> {
    validate_value(value, limits)?;
    let mut encoder = Encoder {
        next_chunk_id,
        ..Encoder::default()
    };
    let mut row = format!("{row_id:x}:");
    encoder.push_value(&mut row, value)?;
    row.push('\n');
    let next_chunk_id = encoder.next_chunk_id;
    let mut chunks = Vec::new();
    chunks.extend(encoder.imports.into_iter().map(String::into_bytes));
    chunks.extend(encoder.binary_rows);
    chunks.extend(encoder.rows.into_iter().map(String::into_bytes));
    chunks.push(row.into_bytes());
    chunks.extend(encoder.deferred_rows.into_iter().map(String::into_bytes));
    Ok((chunks, next_chunk_id))
}

pub fn encode_error(digest: &str) -> Vec<u8> {
    encode_error_for_chunk(0, digest)
}

pub fn encode_error_for_chunk(chunk_id: u32, digest: &str) -> Vec<u8> {
    let mut output = format!(
        "{chunk_id:x}:E{{\"name\":\"Error\",\"message\":\"Rust Server Component render \
         failed\",\"stack\":[],\"env\":\"Server\",\"digest\":"
    );
    push_json_string(&mut output, digest);
    output.push_str("}\n");
    output.into_bytes()
}

#[derive(Default)]
struct Encoder {
    next_chunk_id: u32,
    imports: Vec<String>,
    rows: Vec<String>,
    binary_rows: Vec<Vec<u8>>,
    deferred_rows: Vec<String>,
    written_client_references: BTreeMap<String, u32>,
    written_symbols: BTreeMap<String, u32>,
}

impl Encoder {
    fn allocate_chunk_id(&mut self) -> u32 {
        if self.next_chunk_id == 0 {
            self.next_chunk_id = 1;
        }
        let id = self.next_chunk_id;
        self.next_chunk_id += 1;
        id
    }

    fn push_node(&mut self, output: &mut String, node: &Node) -> Result<(), FlightEncodeError> {
        self.push_node_with_key(output, node, None)
    }

    fn push_node_with_key(
        &mut self,
        output: &mut String,
        node: &Node,
        key: Option<&str>,
    ) -> Result<(), FlightEncodeError> {
        match node {
            Node::Null => output.push_str("null"),
            Node::Text(value) if value.len() >= 1024 => {
                let id = self.allocate_chunk_id();
                self.rows.push(format!("{id:x}:T{:x},{value}", value.len()));
                push_json_string(output, &format!("${id:x}"));
            }
            Node::Text(value) => push_flight_string(output, value),
            Node::Element(element) => self.push_element(output, element, key)?,
            Node::ClientReference(reference) => {
                self.push_client_reference(output, reference, key)?
            }
            Node::Fragment(children) => self.push_children(output, children)?,
            Node::ReactFragment { key, children } => {
                let symbol_id = self.serialize_symbol("react.fragment");
                output.push_str("[\"$\",");
                push_json_string(output, &format!("${symbol_id:x}"));
                output.push(',');
                match key {
                    Some(key) => push_json_string(output, key),
                    None => output.push_str("null"),
                }
                output.push_str(",{\"children\":");
                if children.len() == 1 {
                    self.push_node(output, &children[0])?;
                } else {
                    output.push('[');
                    for (index, child) in children.iter().enumerate() {
                        if index != 0 {
                            output.push(',');
                        }
                        self.push_node(output, child)?;
                    }
                    output.push(']');
                }
                output.push_str("}]");
            }
            Node::Suspense { fallback, content } => {
                let symbol_id = self.serialize_symbol("react.suspense");
                output.push_str("[\"$\",");
                push_json_string(output, &format!("${symbol_id:x}"));
                output.push(',');
                match key {
                    Some(key) => push_json_string(output, key),
                    None => output.push_str("null"),
                }
                output.push_str(",{\"fallback\":");
                self.push_node(output, fallback)?;
                output.push_str(",\"children\":");
                self.push_node(output, content)?;
                output.push_str("}]");
            }
            Node::Slot(id) => return Err(FlightEncodeError::UnresolvedSlot(*id)),
        }
        Ok(())
    }

    fn push_value(
        &mut self,
        output: &mut String,
        value: &FlightValue,
    ) -> Result<(), FlightEncodeError> {
        match value {
            FlightValue::Null => output.push_str("null"),
            FlightValue::Undefined => push_json_string(output, "$undefined"),
            FlightValue::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
            FlightValue::Number(value) if value.is_finite() => write!(output, "{value}").unwrap(),
            FlightValue::Number(value) if value.is_nan() => push_json_string(output, "$NaN"),
            FlightValue::Number(value) if value.is_sign_positive() => {
                push_json_string(output, "$Infinity")
            }
            FlightValue::Number(_) => push_json_string(output, "$-Infinity"),
            FlightValue::String(value) => push_flight_string(output, value),
            FlightValue::BigInt(value) => push_json_string(output, &format!("$n{value}")),
            FlightValue::Date(value) => push_json_string(output, &format!("$D{value}")),
            FlightValue::Array(values) => {
                output.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        output.push(',');
                    }
                    self.push_value(output, value)?;
                }
                output.push(']');
            }
            FlightValue::Object(values) => {
                output.push('{');
                for (index, (name, value)) in values.iter().enumerate() {
                    if index != 0 {
                        output.push(',');
                    }
                    push_json_string(output, name);
                    output.push(':');
                    self.push_value(output, value)?;
                }
                output.push('}');
            }
            FlightValue::Map(entries) => {
                let id = self.allocate_chunk_id();
                push_json_string(output, &format!("$Q{id:x}"));
                let mut row = format!("{id:x}:[");
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index != 0 {
                        row.push(',');
                    }
                    row.push('[');
                    self.push_value(&mut row, key)?;
                    row.push(',');
                    self.push_value(&mut row, value)?;
                    row.push(']');
                }
                row.push_str("]\n");
                self.rows.push(row);
            }
            FlightValue::Set(values) => {
                let id = self.allocate_chunk_id();
                push_json_string(output, &format!("$W{id:x}"));
                let mut row = format!("{id:x}:[");
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        row.push(',');
                    }
                    self.push_value(&mut row, value)?;
                }
                row.push_str("]\n");
                self.rows.push(row);
            }
            FlightValue::Uint8Array(bytes) => {
                self.push_binary(output, BinaryKind::Uint8, bytes);
            }
            FlightValue::Binary(kind, bytes) => {
                self.push_binary(output, *kind, bytes);
            }
            FlightValue::FormData(entries) => {
                let id = self.allocate_chunk_id();
                push_json_string(output, &format!("$K{id:x}"));
                let mut row = format!("{id:x}:[");
                for (index, (name, value)) in entries.iter().enumerate() {
                    if index != 0 {
                        row.push(',');
                    }
                    row.push('[');
                    push_flight_string(&mut row, name);
                    row.push(',');
                    self.push_value(&mut row, value)?;
                    row.push(']');
                }
                row.push_str("]\n");
                self.rows.push(row);
            }
            FlightValue::Iterator(values) => {
                let id = self.allocate_chunk_id();
                push_json_string(output, &format!("$i{id:x}"));
                let mut row = format!("{id:x}:[");
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        row.push(',');
                    }
                    self.push_value(&mut row, value)?;
                }
                row.push_str("]\n");
                self.rows.push(row);
            }
            FlightValue::Blob { mime, chunks } => {
                let id = self.allocate_chunk_id();
                push_json_string(output, &format!("$B{id:x}"));
                let mut row = format!("{id:x}:[");
                push_flight_string(&mut row, mime);
                for chunk in chunks {
                    row.push(',');
                    self.push_binary(&mut row, BinaryKind::Uint8, chunk);
                }
                row.push_str("]\n");
                self.rows.push(row);
            }
            FlightValue::ReadableStream(values) => {
                let id = self.allocate_chunk_id();
                push_json_string(output, &format!("${id:x}"));
                self.rows.push(format!("{id:x}:R\n"));
                for value in values {
                    let mut row = format!("{id:x}:");
                    self.push_value(&mut row, value)?;
                    row.push('\n');
                    self.rows.push(row);
                }
                self.rows.push(format!("{id:x}:C\n"));
            }
            FlightValue::ByteStream(chunks) => {
                let id = self.allocate_chunk_id();
                push_json_string(output, &format!("${id:x}"));
                self.binary_rows.push(format!("{id:x}:r\n").into_bytes());
                for bytes in chunks {
                    let mut row = format!("{id:x}:b{:x},", bytes.len()).into_bytes();
                    row.extend_from_slice(bytes);
                    self.binary_rows.push(row);
                }
                self.binary_rows.push(format!("{id:x}:C\n").into_bytes());
            }
            FlightValue::AsyncIterable {
                iterator,
                values,
                completion,
            } => {
                let id = self.allocate_chunk_id();
                push_json_string(output, &format!("${id:x}"));
                self.rows
                    .push(format!("{id:x}:{}\n", if *iterator { 'x' } else { 'X' }));
                for value in values {
                    let mut row = format!("{id:x}:");
                    self.push_value(&mut row, value)?;
                    row.push('\n');
                    self.rows.push(row);
                }
                if let Some(completion) = completion {
                    let completion_id = self.allocate_chunk_id();
                    let mut completion_row = format!("{completion_id:x}:");
                    self.push_value(&mut completion_row, completion)?;
                    completion_row.push('\n');
                    self.rows.push(completion_row);
                    self.rows.push(format!("{id:x}:C\"${completion_id:x}\"\n"));
                } else {
                    self.rows.push(format!("{id:x}:C\n"));
                }
            }
            FlightValue::Node(node) => self.push_node(output, node)?,
            FlightValue::Deferred(value) => {
                let id = self.allocate_chunk_id();
                push_json_string(output, &format!("$@{id:x}"));
                let mut row = format!("{id:x}:");
                self.push_value(&mut row, value)?;
                row.push('\n');
                self.deferred_rows.push(row);
            }
            FlightValue::Pending(id) => push_json_string(output, &format!("$@{id:x}")),
        }
        Ok(())
    }

    fn serialize_symbol(&mut self, symbol: &str) -> u32 {
        if let Some(id) = self.written_symbols.get(symbol) {
            return *id;
        }
        let id = self.allocate_chunk_id();
        let mut row = format!("{id:x}:");
        push_json_string(&mut row, &format!("$S{symbol}"));
        row.push('\n');
        self.rows.push(row);
        self.written_symbols.insert(symbol.to_owned(), id);
        id
    }

    fn push_binary(&mut self, output: &mut String, kind: BinaryKind, bytes: &[u8]) {
        let id = self.allocate_chunk_id();
        push_json_string(output, &format!("${id:x}"));
        let mut row = format!("{id:x}:{}{:x},", char::from(kind.tag()), bytes.len()).into_bytes();
        row.extend_from_slice(bytes);
        self.binary_rows.push(row);
    }

    fn push_element(
        &mut self,
        output: &mut String,
        element: &Element,
        key: Option<&str>,
    ) -> Result<(), FlightEncodeError> {
        output.push_str("[\"$\",");
        push_json_string(output, &element.tag);
        output.push(',');
        match key {
            Some(key) => push_json_string(output, key),
            None => output.push_str("null"),
        }
        output.push(',');
        self.push_component_props(output, &element.props, &element.children)?;
        output.push(']');
        Ok(())
    }

    fn push_client_reference(
        &mut self,
        output: &mut String,
        reference: &ClientReference,
        element_key: Option<&str>,
    ) -> Result<(), FlightEncodeError> {
        let key = format!(
            "{}#{}#{}",
            reference.module_id, reference.export_name, reference.asynchronous
        );
        let import_id = if let Some(id) = self.written_client_references.get(&key) {
            *id
        } else {
            let id = self.allocate_chunk_id();
            let mut import = format!("{id:x}:I[");
            push_json_string(&mut import, &reference.module_id);
            import.push_str(",[");
            for (index, chunk) in reference.chunks.iter().enumerate() {
                if index != 0 {
                    import.push(',');
                }
                push_json_string(&mut import, chunk);
            }
            import.push_str("],");
            push_json_string(&mut import, &reference.export_name);
            if reference.asynchronous {
                import.push_str(",1");
            }
            import.push_str("]\n");
            self.imports.push(import);
            self.written_client_references.insert(key, id);
            id
        };

        output.push_str("[\"$\",");
        push_json_string(output, &format!("$L{import_id:x}"));
        output.push(',');
        match element_key {
            Some(key) => push_json_string(output, key),
            None => output.push_str("null"),
        }
        output.push(',');
        self.push_component_props(output, &reference.props, &reference.children)?;
        output.push(']');
        Ok(())
    }

    fn push_component_props(
        &mut self,
        output: &mut String,
        props: &BTreeMap<String, PropValue>,
        children: &[Node],
    ) -> Result<(), FlightEncodeError> {
        output.push('{');
        let mut needs_comma = false;
        for (name, value) in props {
            if needs_comma {
                output.push(',');
            }
            needs_comma = true;
            push_json_string(output, name);
            output.push(':');
            self.push_prop_value(output, value)?;
        }

        if !children.is_empty() {
            if needs_comma {
                output.push(',');
            }
            output.push_str("\"children\":");
            if children.len() == 1 {
                self.push_node(output, &children[0])?;
            } else {
                self.push_children(output, children)?;
            }
        }
        output.push('}');
        Ok(())
    }

    fn push_prop_value(
        &mut self,
        output: &mut String,
        value: &PropValue,
    ) -> Result<(), FlightEncodeError> {
        match value {
            PropValue::Undefined => push_json_string(output, "$undefined"),
            PropValue::Node(node) => self.push_node(output, node)?,
            value => push_scalar_prop_value(output, value),
        }
        Ok(())
    }

    fn push_children(
        &mut self,
        output: &mut String,
        children: &[Node],
    ) -> Result<(), FlightEncodeError> {
        output.push('[');
        for (index, child) in children.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            let key = index.to_string();
            self.push_node_with_key(output, child, Some(&key))?;
        }
        output.push(']');
        Ok(())
    }
}

fn push_scalar_prop_value(output: &mut String, value: &PropValue) {
    match value {
        PropValue::Null => output.push_str("null"),
        PropValue::Undefined | PropValue::Node(_) => unreachable!("handled by Encoder"),
        PropValue::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        PropValue::Number(value) if value.is_finite() => write!(output, "{value}").unwrap(),
        PropValue::Number(value) if value.is_nan() => push_json_string(output, "$NaN"),
        PropValue::Number(value) if value.is_sign_positive() => {
            push_json_string(output, "$Infinity")
        }
        PropValue::Number(_) => push_json_string(output, "$-Infinity"),
        PropValue::String(value) => push_flight_string(output, value),
        PropValue::Strings(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                push_flight_string(output, value);
            }
            output.push(']');
        }
        PropValue::Style(values) => {
            output.push('{');
            for (index, (name, value)) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                push_json_string(output, name);
                output.push(':');
                push_flight_string(output, value);
            }
            output.push('}');
        }
    }
}

fn push_flight_string(output: &mut String, value: &str) {
    if value.starts_with('$') {
        let mut escaped = String::with_capacity(value.len() + 1);
        escaped.push('$');
        escaped.push_str(value);
        push_json_string(output, &escaped);
    } else {
        push_json_string(output, value);
    }
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch <= '\u{1f}' => write!(output, "\\u{:04x}", ch as u32).unwrap(),
            ch => output.push(ch),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use next_rsc::{LayoutProps, Node, client_reference, element};

    use super::{
        BinaryKind, ConsoleMethod, DevComponentInfo, DevStackFrame, EncodeLimits,
        FlightEncodeError, FlightSink, FlightTaskGraph, FlightValue, HintCode, SinkStatus,
        encode_dev_component_chunks, encode_dev_console_chunks, encode_dev_time_origin,
        encode_dev_timing, encode_error, encode_hint_chunks, encode_root, encode_root_chunks,
        encode_root_chunks_with_limits, encode_root_value, encode_task_resolution_chunks,
        write_root_value_with_cancel,
    };

    #[test]
    fn emits_resource_hint_rows_for_the_pinned_protocol() {
        assert_eq!(
            encode_hint_chunks(
                HintCode::DnsPrefetch,
                &FlightValue::from("https://assets.example")
            )
            .unwrap(),
            vec![b":HD\"https://assets.example\"\n".to_vec()]
        );
        assert_eq!(
            encode_hint_chunks(
                HintCode::Style,
                &FlightValue::Array(vec![
                    FlightValue::from("/app.css"),
                    FlightValue::from("high"),
                ])
            )
            .unwrap(),
            vec![b":HS[\"/app.css\",\"high\"]\n".to_vec()]
        );
    }

    #[test]
    fn emits_async_iterable_and_completion_rows() {
        let value = FlightValue::AsyncIterable {
            iterator: true,
            values: vec![FlightValue::from("one"), FlightValue::from("two")],
            completion: Some(Box::new(FlightValue::from("done"))),
        };
        assert_eq!(
            encode_root_chunks(&value).unwrap(),
            vec![
                b"1:x\n".to_vec(),
                b"1:\"one\"\n".to_vec(),
                b"1:\"two\"\n".to_vec(),
                b"2:\"done\"\n".to_vec(),
                b"1:C\"$2\"\n".to_vec(),
                b"0:\"$1\"\n".to_vec(),
            ]
        );
    }

    #[test]
    fn emits_development_timing_component_and_console_rows() {
        assert_eq!(encode_dev_time_origin(12.5), b":N12.5\n");
        assert_eq!(encode_dev_timing(0, 1.25), b"0:D{\"time\":1.25}\n");
        let frame = DevStackFrame {
            name: "RustPage".to_owned(),
            file: "app/page.rs".to_owned(),
            line: 3,
            column: 1,
        };
        assert_eq!(
            encode_dev_component_chunks(
                0,
                1,
                2,
                &DevComponentInfo {
                    name: "RustPage".to_owned(),
                    env: "Server".to_owned(),
                    stack: vec![frame.clone()],
                }
            ),
            vec![
                b"1:{\"name\":\"RustPage\",\"key\":null,\"env\":\"Server\",\"stack\":[[\"RustPage\",\"app/page.rs\",3,1,3,1,false]],\"props\":{}}\n".to_vec(),
                b"2:[[\"RustPage\",\"app/page.rs\",3,1,3,1,false]]\n".to_vec(),
                b"0:D\"$1\"\n".to_vec(),
            ]
        );
        assert_eq!(
            encode_dev_console_chunks(
                ConsoleMethod::Warn,
                &[frame],
                "Server",
                &[FlightValue::from("slow query")]
            )
            .unwrap(),
            vec![
                b":W[\"warn\",[[\"RustPage\",\"app/page.rs\",3,1,3,1,false]],null,\"Server\",\"slow query\"]\n"
                    .to_vec()
            ]
        );
    }

    #[test]
    fn encodes_intrinsic_element_model() {
        let model =
            element("main", [Node::text("hello"), Node::text("$value")]).prop("className", "shell");
        assert_eq!(
            encode_root(&model).unwrap(),
            b"0:[\"$\",\"main\",null,{\"className\":\"shell\",\"children\":[\"hello\",\"$$value\"]}]\n"
        );
    }

    #[test]
    fn refuses_unresolved_js_slot() {
        assert_eq!(
            encode_root(&LayoutProps::prototype().children),
            Err(FlightEncodeError::UnresolvedSlot(next_rsc::SlotId(0)))
        );
    }

    #[test]
    fn rejects_values_over_explicit_resource_limits() {
        let tiny = EncodeLimits {
            max_depth: 3,
            max_items: 10,
            max_string_bytes: 4,
            max_chunks: 2,
            max_buffered_bytes: 32,
        };
        assert_eq!(
            encode_root_chunks_with_limits(&FlightValue::String("12345".to_owned()), tiny),
            Err(FlightEncodeError::LimitExceeded("string bytes"))
        );

        let nested = FlightValue::Array(vec![FlightValue::Array(vec![FlightValue::Array(vec![
            FlightValue::Array(vec![FlightValue::Null]),
        ])])]);
        assert_eq!(
            encode_root_chunks_with_limits(&nested, tiny),
            Err(FlightEncodeError::LimitExceeded("nesting depth"))
        );
    }

    #[test]
    fn emits_and_deduplicates_client_import_chunks() {
        let button =
            || client_reference("42", "default", ["7", "button.js"], []).prop("label", "press");
        let model = element("main", [button(), button()]);
        assert_eq!(
            String::from_utf8(encode_root(&model).unwrap()).unwrap(),
            concat!(
                "1:I[\"42\",[\"7\",\"button.js\"],\"default\"]\n",
                "0:[\"$\",\"main\",null,{\"children\":[",
                "[\"$\",\"$L1\",\"0\",{\"label\":\"press\"}],",
                "[\"$\",\"$L1\",\"1\",{\"label\":\"press\"}]",
                "]}]\n"
            )
        );
    }

    #[test]
    fn encodes_next_payload_values_with_embedded_nodes() {
        let payload = FlightValue::object([
            (
                "f",
                FlightValue::Array(vec![FlightValue::Node(element("main", []))]),
            ),
            ("S", FlightValue::Bool(false)),
            ("r", FlightValue::Undefined),
        ]);
        assert_eq!(
            String::from_utf8(encode_root_value(&payload).unwrap()).unwrap(),
            "0:{\"S\":false,\"f\":[[\"$\",\"main\",null,{}]],\"r\":\"$undefined\"}\n"
        );
    }

    #[test]
    fn outlines_large_text_as_an_atomic_text_row() {
        let text = "x".repeat(1024);
        let chunks = encode_root_chunks(&FlightValue::Node(Node::text(&text))).unwrap();
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with(b"1:T400,"));
        assert_eq!(chunks[0].len(), 7 + 1024);
        assert_eq!(chunks[1], b"0:\"$1\"\n");
    }

    #[test]
    fn encodes_root_error_row() {
        assert_eq!(
            encode_error("rust-render-failed"),
            b"0:E{\"name\":\"Error\",\"message\":\"Rust Server Component render failed\",\"stack\":[],\"env\":\"Server\",\"digest\":\"rust-render-failed\"}\n"
        );
    }

    #[test]
    fn emits_root_before_deferred_resolution() {
        let payload = FlightValue::object([(
            "later",
            FlightValue::Deferred(Box::new(FlightValue::String("ready".to_owned()))),
        )]);
        let chunks = encode_root_chunks(&payload).unwrap();
        assert_eq!(
            chunks,
            vec![
                b"0:{\"later\":\"$@1\"}\n".to_vec(),
                b"1:\"ready\"\n".to_vec(),
            ]
        );
    }

    #[test]
    fn resolves_an_explicit_pending_task_in_a_later_batch() {
        let root = FlightValue::object([("node", FlightValue::Pending(0x100))]);
        assert_eq!(
            encode_root_chunks(&root).unwrap(),
            vec![b"0:{\"node\":\"$@100\"}\n".to_vec()]
        );
        assert_eq!(
            encode_task_resolution_chunks(
                0x100,
                &FlightValue::Node(element("p", [Node::text("ready")]))
            )
            .unwrap(),
            vec![b"100:[\"$\",\"p\",null,{\"children\":\"ready\"}]\n".to_vec()]
        );
    }

    #[test]
    fn emits_react_suspense_symbol() {
        let model = Node::suspense(Node::text("loading"), Node::text("done"));
        assert_eq!(
            String::from_utf8(encode_root(&model).unwrap()).unwrap(),
            concat!(
                "1:\"$Sreact.suspense\"\n",
                "0:[\"$\",\"$1\",null,{\"fallback\":\"loading\",\"children\":\"done\"}]\n"
            )
        );
    }

    #[test]
    fn cancellation_stops_between_chunks() {
        let payload = FlightValue::Deferred(Box::new(FlightValue::String("ready".to_owned())));
        let mut writes = Vec::new();
        let mut checks = 0;
        let error = write_root_value_with_cancel(&mut writes, &payload, || {
            checks += 1;
            checks > 1
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "Flight stream cancelled");
        assert_eq!(writes, b"0:\"$@1\"\n");
    }

    #[derive(Default)]
    struct TestSink {
        writes: Vec<Vec<u8>>,
        block_next: bool,
    }

    impl FlightSink for TestSink {
        type Error = std::convert::Infallible;

        fn write_chunk(&mut self, chunk: &[u8]) -> Result<SinkStatus, Self::Error> {
            if self.block_next {
                self.block_next = false;
                return Ok(SinkStatus::Backpressure);
            }
            self.writes.push(chunk.to_vec());
            Ok(SinkStatus::Ready)
        }
    }

    #[test]
    fn task_graph_assigns_monotonic_ids_and_resolves_out_of_order() {
        let mut graph = FlightTaskGraph::new(EncodeLimits::default());
        let (first_id, first) = graph.reserve_task().unwrap();
        let (second_id, second) = graph.reserve_task().unwrap();
        assert_eq!((first_id, second_id), (1, 2));
        graph
            .enqueue_root(&FlightValue::object([("first", first), ("second", second)]))
            .unwrap();
        graph
            .resolve_task(second_id, &FlightValue::from("second ready"))
            .unwrap();
        graph
            .resolve_task(first_id, &FlightValue::from("first ready"))
            .unwrap();

        let mut sink = TestSink::default();
        assert_eq!(graph.drain_to_sink(&mut sink).unwrap(), SinkStatus::Ready);
        assert_eq!(
            sink.writes,
            vec![
                b"0:{\"first\":\"$@1\",\"second\":\"$@2\"}\n".to_vec(),
                b"2:\"second ready\"\n".to_vec(),
                b"1:\"first ready\"\n".to_vec(),
            ]
        );
    }

    #[test]
    fn task_graph_resumes_after_backpressure_without_loss_or_duplication() {
        let mut graph = FlightTaskGraph::new(EncodeLimits::default());
        let (task_id, pending) = graph.reserve_task().unwrap();
        graph.enqueue_root(&pending).unwrap();
        graph
            .resolve_task(task_id, &FlightValue::from("ready"))
            .unwrap();
        let queued_bytes = graph.queued_bytes();
        let mut sink = TestSink {
            block_next: true,
            ..TestSink::default()
        };
        assert_eq!(
            graph.drain_to_sink(&mut sink).unwrap(),
            SinkStatus::Backpressure
        );
        assert_eq!(graph.queued_bytes(), queued_bytes);
        assert!(sink.writes.is_empty());
        assert_eq!(graph.drain_to_sink(&mut sink).unwrap(), SinkStatus::Ready);
        assert_eq!(
            sink.writes,
            vec![b"0:\"$@1\"\n".to_vec(), b"1:\"ready\"\n".to_vec()]
        );
        assert_eq!(graph.queued_bytes(), 0);
    }

    #[test]
    fn task_graph_enforces_live_queue_bounds_and_cancels() {
        let limits = EncodeLimits {
            max_buffered_bytes: 8,
            ..EncodeLimits::default()
        };
        let mut bounded = FlightTaskGraph::new(limits);
        assert_eq!(
            bounded.enqueue_root(&FlightValue::from("\"\"\"\"")),
            Err(FlightEncodeError::LimitExceeded("buffered bytes"))
        );

        let mut cancelled = FlightTaskGraph::new(EncodeLimits::default());
        let (task_id, pending) = cancelled.reserve_task().unwrap();
        cancelled.enqueue_root(&pending).unwrap();
        assert!(cancelled.queued_bytes() > 0);
        cancelled.cancel();
        assert!(cancelled.is_cancelled());
        assert_eq!(cancelled.queued_bytes(), 0);
        assert_eq!(
            cancelled.resolve_task(task_id, &FlightValue::from("late")),
            Err(FlightEncodeError::InvalidTask("request cancelled"))
        );
    }

    #[test]
    fn bounded_generated_models_never_panic_and_encode_deterministically() {
        fn next(seed: &mut u64) -> u64 {
            *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            *seed
        }
        fn model(seed: &mut u64, depth: usize) -> FlightValue {
            if depth == 0 {
                return match next(seed) % 5 {
                    0 => FlightValue::Null,
                    1 => FlightValue::Bool(next(seed) & 1 == 1),
                    2 => FlightValue::Number((next(seed) % 10_000) as f64 / 10.0),
                    3 => FlightValue::from(format!("$generated-{}", next(seed) % 100)),
                    _ => FlightValue::Binary(BinaryKind::Uint8, next(seed).to_le_bytes().to_vec()),
                };
            }
            match next(seed) % 6 {
                0 => FlightValue::Array(
                    (0..next(seed) % 4)
                        .map(|_| model(seed, depth - 1))
                        .collect(),
                ),
                1 => FlightValue::object([
                    ("left", model(seed, depth - 1)),
                    ("right", model(seed, depth - 1)),
                ]),
                2 => FlightValue::Map(vec![(model(seed, depth - 1), model(seed, depth - 1))]),
                3 => FlightValue::Set(vec![model(seed, depth - 1)]),
                4 => FlightValue::Deferred(Box::new(model(seed, depth - 1))),
                _ => FlightValue::Node(element(
                    "span",
                    [Node::text(format!("generated-{}", next(seed) % 1000))],
                )),
            }
        }

        let mut seed = 0x5eed_cafe_f00d_u64;
        for _ in 0..256 {
            let value = model(&mut seed, 5);
            let first = encode_root_chunks(&value).unwrap();
            let second = encode_root_chunks(&value).unwrap();
            assert_eq!(first, second);
        }
    }
}
