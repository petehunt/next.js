use bytes::{Bytes, BytesMut};
use next_rs_react::{MARKER_PREFIX, MAX_MARKER_PAYLOAD_LEN, is_payload_byte};

/// What the scanner found in the byte stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    /// Bytes that are not part of a marker and pass through untouched.
    Literal(Bytes),
    /// A complete marker payload — the bytes between the delimiters.
    Marker(String),
}

/// Streaming scanner for slot markers (spec §33).
///
/// It does not parse HTML (spec §32): it only recognises `~NRS1.<payload>~`. When
/// a marker straddles a chunk boundary the scanner keeps just enough carry-over
/// bytes to decide, never the whole document (spec §52).
#[derive(Debug)]
pub struct MarkerScanner {
    state: State,
    /// Bytes belonging to a marker candidate that has not resolved yet.
    carry: BytesMut,
    max_payload_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Outside a marker.
    Text,
    /// Matched `n` bytes of the prefix so far (`0 < n < MARKER_PREFIX.len()`).
    Prefix(usize),
    /// Inside the payload; `carry` holds the payload bytes only.
    Payload,
}

impl Default for MarkerScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkerScanner {
    pub fn new() -> Self {
        Self::with_max_payload_len(MAX_MARKER_PAYLOAD_LEN)
    }

    /// Overrides the payload bound, mainly so tests can exercise the overflow
    /// path cheaply.
    pub fn with_max_payload_len(max_payload_len: usize) -> Self {
        Self {
            state: State::Text,
            carry: BytesMut::new(),
            max_payload_len,
        }
    }

    /// Bytes currently held back waiting for more input.
    pub fn carry_len(&self) -> usize {
        self.carry.len()
    }

    /// Feeds a chunk, appending everything it resolves to `events`.
    pub fn feed(&mut self, chunk: &[u8], events: &mut Vec<ScanEvent>) {
        let prefix = MARKER_PREFIX.as_bytes();
        let mut index = 0;

        while index < chunk.len() {
            match self.state {
                State::Text => {
                    // Fast-forward to the next candidate opener.
                    match memchr::memchr(prefix[0], &chunk[index..]) {
                        Some(offset) => {
                            if offset > 0 {
                                push_literal(events, &chunk[index..index + offset]);
                            }
                            index += offset + 1;
                            self.carry.clear();
                            self.carry.extend_from_slice(&prefix[..1]);
                            self.state = State::Prefix(1);
                        }
                        None => {
                            push_literal(events, &chunk[index..]);
                            index = chunk.len();
                        }
                    }
                }
                State::Prefix(matched) => {
                    let byte = chunk[index];
                    if byte == prefix[matched] {
                        index += 1;
                        self.carry.extend_from_slice(&[byte]);
                        if matched + 1 == prefix.len() {
                            self.state = State::Payload;
                            self.carry.clear();
                        } else {
                            self.state = State::Prefix(matched + 1);
                        }
                    } else {
                        // Not a marker after all. Emit what we buffered and
                        // reconsider this byte from `Text` — it may itself be an
                        // opener, as in `~~NRS1.…~`.
                        events.push(ScanEvent::Literal(self.take_carry()));
                        self.state = State::Text;
                    }
                }
                State::Payload => {
                    let rest = &chunk[index..];
                    let terminator = MARKER_TERMINATOR_BYTE;
                    let stop = rest
                        .iter()
                        .position(|byte| *byte == terminator || !is_payload_byte(*byte));
                    match stop {
                        Some(offset) if rest[offset] == terminator => {
                            self.carry.extend_from_slice(&rest[..offset]);
                            index += offset + 1;
                            let payload = self.take_carry();
                            match String::from_utf8(payload.to_vec()) {
                                // Payload bytes are base64url, so this always
                                // succeeds; the fallback keeps the scanner from
                                // panicking on a hostile stream.
                                Ok(payload) => events.push(ScanEvent::Marker(payload)),
                                Err(_) => {
                                    push_literal(events, MARKER_PREFIX.as_bytes());
                                    events.push(ScanEvent::Literal(payload));
                                    push_literal(events, &[terminator]);
                                }
                            }
                            self.state = State::Text;
                        }
                        Some(offset) => {
                            // An illegal payload byte: this was never a marker.
                            self.carry.extend_from_slice(&rest[..offset]);
                            self.emit_failed_candidate(events);
                            index += offset;
                            self.state = State::Text;
                        }
                        None => {
                            self.carry.extend_from_slice(rest);
                            index = chunk.len();
                            if self.carry.len() > self.max_payload_len {
                                self.emit_failed_candidate(events);
                                self.state = State::Text;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Flushes any unresolved candidate at end of stream as literal bytes.
    ///
    /// A truncated marker is ordinary text: better to render it than to drop
    /// bytes the producer wrote.
    pub fn finish(&mut self, events: &mut Vec<ScanEvent>) {
        match self.state {
            State::Text => {}
            State::Prefix(_) => events.push(ScanEvent::Literal(self.take_carry())),
            State::Payload => self.emit_failed_candidate(events),
        }
        self.state = State::Text;
    }

    /// Emits `MARKER_PREFIX` plus the buffered payload as literal text.
    fn emit_failed_candidate(&mut self, events: &mut Vec<ScanEvent>) {
        push_literal(events, MARKER_PREFIX.as_bytes());
        let payload = self.take_carry();
        if !payload.is_empty() {
            events.push(ScanEvent::Literal(payload));
        }
    }

    fn take_carry(&mut self) -> Bytes {
        std::mem::take(&mut self.carry).freeze()
    }
}

const MARKER_TERMINATOR_BYTE: u8 = b'~';

fn push_literal(events: &mut Vec<ScanEvent>, bytes: &[u8]) {
    if !bytes.is_empty() {
        events.push(ScanEvent::Literal(Bytes::copy_from_slice(bytes)));
    }
}

/// Holds back the closing document tail so outstanding slot frames can be
/// emitted before `</body></html>` (spec §51).
///
/// Only the tail is retained; every prior byte flushes immediately.
#[derive(Debug, Default)]
pub struct TailGuard {
    triggered: bool,
    held: BytesMut,
    /// A suffix that could be the start of a closing tag.
    partial: BytesMut,
}

/// Closing tags that begin the document tail, longest first so `</body` wins
/// when both appear at the same position.
const TAIL_MARKERS: [&[u8]; 2] = [b"</body", b"</html"];

impl TailGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// True once the closing tail has been seen.
    pub fn is_holding(&self) -> bool {
        self.triggered
    }

    pub fn held_len(&self) -> usize {
        self.held.len() + self.partial.len()
    }

    /// Feeds document bytes, returning the bytes safe to emit now.
    pub fn feed(&mut self, chunk: &[u8]) -> Option<Bytes> {
        if self.triggered {
            self.held.extend_from_slice(chunk);
            return None;
        }

        let mut buffer = std::mem::take(&mut self.partial);
        buffer.extend_from_slice(chunk);

        match find_tail(&buffer) {
            Some(index) => {
                self.triggered = true;
                let emit = buffer.split_to(index).freeze();
                self.held = buffer;
                (!emit.is_empty()).then_some(emit)
            }
            None => {
                let keep = partial_tail_len(&buffer);
                let split = buffer.len() - keep;
                let emit = buffer.split_to(split).freeze();
                self.partial = buffer;
                (!emit.is_empty()).then_some(emit)
            }
        }
    }

    /// Releases everything held back.
    pub fn finish(&mut self) -> Option<Bytes> {
        let mut out = std::mem::take(&mut self.partial);
        out.unsplit(std::mem::take(&mut self.held));
        self.triggered = false;
        (!out.is_empty()).then(|| out.freeze())
    }
}

fn find_tail(buffer: &[u8]) -> Option<usize> {
    TAIL_MARKERS
        .iter()
        .filter_map(|marker| find_ignore_ascii_case(buffer, marker))
        .min()
}

fn find_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

/// Length of the longest suffix of `buffer` that is a prefix of a tail marker.
fn partial_tail_len(buffer: &[u8]) -> usize {
    let longest = TAIL_MARKERS
        .iter()
        .map(|marker| marker.len())
        .max()
        .unwrap_or(0);
    let max = longest.saturating_sub(1).min(buffer.len());
    for len in (1..=max).rev() {
        let suffix = &buffer[buffer.len() - len..];
        if TAIL_MARKERS
            .iter()
            .any(|marker| marker[..len].eq_ignore_ascii_case(suffix))
        {
            return len;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&str]) -> Vec<ScanEvent> {
        let mut scanner = MarkerScanner::new();
        let mut events = Vec::new();
        for chunk in chunks {
            scanner.feed(chunk.as_bytes(), &mut events);
        }
        scanner.finish(&mut events);
        events
    }

    fn literals(events: &[ScanEvent]) -> String {
        events
            .iter()
            .map(|event| match event {
                ScanEvent::Literal(bytes) => String::from_utf8_lossy(bytes).into_owned(),
                ScanEvent::Marker(payload) => format!("~NRS1.{payload}~"),
            })
            .collect()
    }

    fn markers(events: &[ScanEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| match event {
                ScanEvent::Marker(payload) => Some(payload.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn passes_through_plain_html() {
        let events = scan(&["<h1>hello</h1>"]);
        assert_eq!(markers(&events), Vec::<&str>::new());
        assert_eq!(literals(&events), "<h1>hello</h1>");
    }

    #[test]
    fn finds_a_marker_in_one_chunk() {
        let events = scan(&["<main>~NRS1.abc123~</main>"]);
        assert_eq!(markers(&events), vec!["abc123"]);
        assert_eq!(literals(&events), "<main>~NRS1.abc123~</main>");
    }

    #[test]
    fn finds_a_marker_split_across_chunks() {
        // The example from spec §33.
        let events = scan(&["<main>~NRS1.ab", "cdef123~</main>"]);
        assert_eq!(markers(&events), vec!["abcdef123"]);
    }

    #[test]
    fn finds_a_marker_split_inside_the_prefix() {
        let events = scan(&["<main>~NR", "S1.abc~"]);
        assert_eq!(markers(&events), vec!["abc"]);
    }

    #[test]
    fn handles_one_byte_at_a_time() {
        let input = "a~NRS1.xyz~b~NRS1.w~c";
        let chunks: Vec<String> = input.chars().map(|c| c.to_string()).collect();
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let events = scan(&refs);
        assert_eq!(markers(&events), vec!["xyz", "w"]);
        assert_eq!(literals(&events), input);
    }

    #[test]
    fn finds_adjacent_markers() {
        let events = scan(&["~NRS1.a~~NRS1.b~"]);
        assert_eq!(markers(&events), vec!["a", "b"]);
    }

    #[test]
    fn a_tilde_that_is_not_a_marker_is_literal() {
        let events = scan(&["~ ~NRS ~NRS1x~NRS1.ok~"]);
        assert_eq!(markers(&events), vec!["ok"]);
        assert_eq!(literals(&events), "~ ~NRS ~NRS1x~NRS1.ok~");
    }

    #[test]
    fn a_double_tilde_still_opens_a_marker() {
        let events = scan(&["~~NRS1.ok~"]);
        assert_eq!(markers(&events), vec!["ok"]);
        assert_eq!(literals(&events), "~~NRS1.ok~");
    }

    #[test]
    fn an_empty_payload_is_still_a_marker() {
        // `MarkerSecret::marker` emits this when sealing fails, so the
        // transformer must see it and report a slot error.
        let events = scan(&["~NRS1.~"]);
        assert_eq!(markers(&events), vec![""]);
    }

    #[test]
    fn illegal_payload_bytes_abandon_the_candidate() {
        let events = scan(&["~NRS1.ab<c~"]);
        assert!(markers(&events).is_empty());
        assert_eq!(literals(&events), "~NRS1.ab<c~");
    }

    #[test]
    fn a_truncated_marker_at_eof_is_literal() {
        let events = scan(&["<p>~NRS1.abc"]);
        assert!(markers(&events).is_empty());
        assert_eq!(literals(&events), "<p>~NRS1.abc");

        let events = scan(&["<p>~NR"]);
        assert_eq!(literals(&events), "<p>~NR");
    }

    #[test]
    fn an_oversized_payload_is_abandoned_without_unbounded_buffering() {
        let mut scanner = MarkerScanner::with_max_payload_len(16);
        let mut events = Vec::new();
        scanner.feed(b"~NRS1.", &mut events);
        scanner.feed(&[b'A'; 64], &mut events);
        assert!(scanner.carry_len() <= 16 + 1);
        scanner.feed(b"~", &mut events);
        scanner.finish(&mut events);
        assert!(markers(&events).is_empty());
        assert!(literals(&events).starts_with("~NRS1.AAAA"));
    }

    #[test]
    fn a_payload_exactly_at_the_bound_is_still_accepted() {
        let mut scanner = MarkerScanner::with_max_payload_len(16);
        let mut events = Vec::new();
        scanner.feed(b"~NRS1.", &mut events);
        scanner.feed(&[b'A'; 16], &mut events);
        scanner.feed(b"~", &mut events);
        scanner.finish(&mut events);
        assert_eq!(markers(&events), vec!["A".repeat(16)]);
    }

    #[test]
    fn carry_stays_bounded_for_ordinary_html() {
        let mut scanner = MarkerScanner::new();
        let mut events = Vec::new();
        for _ in 0..1000 {
            scanner.feed(b"<div>lots of text</div>", &mut events);
        }
        assert_eq!(scanner.carry_len(), 0);
    }

    #[test]
    fn multibyte_utf8_passes_through_untouched() {
        let events = scan(&["<p>héllo → 世界</p>~NRS1.a~"]);
        assert_eq!(markers(&events), vec!["a"]);
        assert_eq!(literals(&events), "<p>héllo → 世界</p>~NRS1.a~");
    }

    #[test]
    fn tail_guard_holds_the_closing_tags() {
        let mut guard = TailGuard::new();
        assert_eq!(
            guard.feed(b"<html><body>hi").map(|b| b.to_vec()),
            Some(b"<html><body>hi".to_vec())
        );
        assert!(!guard.is_holding());
        assert_eq!(guard.feed(b"</body></html>"), None);
        assert!(guard.is_holding());
        assert_eq!(
            guard.finish().map(|b| b.to_vec()),
            Some(b"</body></html>".to_vec())
        );
    }

    #[test]
    fn tail_guard_splits_a_chunk_at_the_closing_tag() {
        let mut guard = TailGuard::new();
        let emitted = guard.feed(b"<p>x</p></body></html>").unwrap();
        assert_eq!(emitted.to_vec(), b"<p>x</p>".to_vec());
        assert_eq!(guard.finish().unwrap().to_vec(), b"</body></html>".to_vec());
    }

    #[test]
    fn tail_guard_handles_a_split_closing_tag() {
        let mut guard = TailGuard::new();
        // `</bo` could be the start of `</body`, so it is held back.
        assert_eq!(
            guard.feed(b"<p>x</p></bo").unwrap().to_vec(),
            b"<p>x</p>".to_vec()
        );
        assert!(!guard.is_holding());
        assert_eq!(guard.feed(b"dy></html>"), None);
        assert!(guard.is_holding());
        assert_eq!(guard.finish().unwrap().to_vec(), b"</body></html>".to_vec());
    }

    #[test]
    fn tail_guard_releases_a_false_partial_match() {
        let mut guard = TailGuard::new();
        assert_eq!(guard.feed(b"a</b").unwrap().to_vec(), b"a".to_vec());
        // `</b>` is a bold close, not a body close.
        assert_eq!(guard.feed(b">c").unwrap().to_vec(), b"</b>c".to_vec());
        assert!(!guard.is_holding());
        assert!(guard.finish().is_none());
    }

    #[test]
    fn tail_guard_is_case_insensitive() {
        let mut guard = TailGuard::new();
        assert!(guard.feed(b"x</BODY></HTML>").is_some());
        assert!(guard.is_holding());
        assert_eq!(guard.finish().unwrap().to_vec(), b"</BODY></HTML>".to_vec());
    }

    #[test]
    fn tail_guard_uses_html_close_when_there_is_no_body() {
        let mut guard = TailGuard::new();
        assert_eq!(
            guard.feed(b"<html>x</html>").unwrap().to_vec(),
            b"<html>x".to_vec()
        );
        assert!(guard.is_holding());
    }

    #[test]
    fn tail_guard_holds_nothing_for_a_fragment() {
        let mut guard = TailGuard::new();
        assert_eq!(
            guard.feed(b"<div>fragment</div>").unwrap().to_vec(),
            b"<div>fragment</div>".to_vec()
        );
        assert!(guard.finish().is_none());
        assert_eq!(guard.held_len(), 0);
    }

    #[test]
    fn partial_tail_len_finds_the_longest_prefix() {
        assert_eq!(partial_tail_len(b"abc"), 0);
        assert_eq!(partial_tail_len(b"abc<"), 1);
        assert_eq!(partial_tail_len(b"abc</"), 2);
        assert_eq!(partial_tail_len(b"abc</b"), 3);
        assert_eq!(partial_tail_len(b"abc</BOD"), 5);
        // A complete marker is detected by `find_tail`, not held as partial.
        assert_eq!(partial_tail_len(b"</bodyx"), 0);
    }
}
