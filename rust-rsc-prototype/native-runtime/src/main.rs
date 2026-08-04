use std::{
    collections::BTreeMap,
    env,
    fmt::Write as _,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use next_rsc::{Node, PropValue, RenderError};

mod catalog;
include!("generated_routes.rs");

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_ACTIVE_REQUESTS: usize = 128;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
static ACTIVE_REQUESTS: AtomicUsize = AtomicUsize::new(0);
static NEXT_REQUEST_ID: AtomicUsize = AtomicUsize::new(1);

struct ActiveRequest;

impl ActiveRequest {
    fn try_acquire() -> Option<Self> {
        ACTIVE_REQUESTS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE_REQUESTS).then_some(active + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        ACTIVE_REQUESTS.fetch_sub(1, Ordering::Release);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let port = env::var("PORT").unwrap_or_else(|_| "3030".to_owned());
    let listener = TcpListener::bind(format!("{host}:{port}"))?;
    listener.set_nonblocking(true)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown))?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&shutdown))?;
    println!("Rust RSC native runtime listening on http://{host}:{port}");
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _peer)) => {
                let Some(active) = ActiveRequest::try_acquire() else {
                    let _ = write_response(
                        &mut stream,
                        "503 Service Unavailable",
                        "text/plain",
                        b"Native runtime is at capacity",
                    );
                    continue;
                };
                let _ = thread::Builder::new()
                    .name("rust-rsc-request".to_owned())
                    .spawn(move || {
                        let _active = active;
                        if let Err(error) = handle_request(&mut stream) {
                            eprintln!("request failed: {error}");
                        }
                    });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => eprintln!("accept failed: {error}"),
        }
    }
    while ACTIVE_REQUESTS.load(Ordering::Acquire) != 0 {
        thread::sleep(Duration::from_millis(5));
    }
    println!("Rust RSC native runtime shut down cleanly");
    Ok(())
}

fn handle_request(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let started = Instant::now();
    let result = handle_request_inner(stream, request_id);
    if env::var_os("RUST_RSC_TRACE").is_some() {
        eprintln!(
            "rust_rsc request_complete id={request_id} elapsed_us={} result={}",
            started.elapsed().as_micros(),
            if result.is_ok() { "ok" } else { "error" }
        );
    }
    result
}

fn handle_request_inner(
    stream: &mut TcpStream,
    request_id: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let request = match read_request(stream) {
        Ok(request) => request,
        Err((status, message)) => {
            return write_response(stream, status, "text/plain", message.as_bytes());
        }
    };
    let headers_end = find_bytes(&request, b"\r\n\r\n").expect("validated request headers");
    let header_text = std::str::from_utf8(&request[..headers_end])?;
    let mut lines = header_text.split("\r\n");
    let first_line = lines.next().unwrap_or_default();
    let mut request_parts = first_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default();
    let request_target = request_parts.next().unwrap_or("/");
    if method.is_empty()
        || !request_target.starts_with('/')
        || request_target.contains('#')
        || request_parts.next() != Some("HTTP/1.1")
        || request_parts.next().is_some()
    {
        return write_response(
            stream,
            "400 Bad Request",
            "text/plain",
            b"Malformed request line",
        );
    }
    let headers: Vec<_> = lines.filter_map(|line| line.split_once(':')).collect();
    let request_data = collect_request_data(&headers);
    let request_kind = match classify_request(&headers) {
        Ok(kind) => kind,
        Err(message) => {
            return write_response(stream, "400 Bad Request", "text/plain", message.as_bytes());
        }
    };
    let segment_prefetch_key = headers.iter().find_map(|(name, value)| {
        name.eq_ignore_ascii_case("next-router-segment-prefetch")
            .then_some(value.trim())
    });
    trace_request_selection(request_id, method, request_target, request_kind, "classify");
    let raw_pathname = request_target.split('?').next().unwrap_or(request_target);
    let Some(mut pathname) = normalize_request_path(raw_pathname) else {
        if let Ok(fallback) = env::var("NEXT_FALLBACK_ADDR") {
            trace_request_selection(request_id, method, request_target, request_kind, "fallback");
            return proxy_to_fallback(stream, &fallback, &request, headers_end);
        }
        return write_response(stream, "404 Not Found", "text/plain", b"Not found");
    };
    match apply_native_rewrite(&pathname) {
        NativeRewrite::Internal(destination) => pathname = destination,
        NativeRewrite::Fallback => {
            if let Ok(fallback) = env::var("NEXT_FALLBACK_ADDR") {
                trace_request_selection(
                    request_id,
                    method,
                    request_target,
                    request_kind,
                    "fallback-rewrite",
                );
                return proxy_to_fallback(stream, &fallback, &request, headers_end);
            }
            return write_response(
                stream,
                "501 Not Implemented",
                "text/plain",
                b"Rewrite requires the Next.js fallback",
            );
        }
        NativeRewrite::None => {}
    }
    let pathname = pathname.as_str();
    if pathname.starts_with("/_next/static/") {
        if method != "GET" && method != "HEAD" {
            return write_method_not_allowed(stream);
        }
        return serve_static_asset(stream, pathname, method);
    }
    if let Some(filename) = public_asset_path(pathname) {
        if method != "GET" && method != "HEAD" {
            return write_method_not_allowed(stream);
        }
        let body = fs::read(&filename)?;
        return write_response_for_method(
            stream,
            "200 OK",
            asset_content_type(&filename),
            &body,
            method,
        );
    }
    if let Some(location) = trailing_slash_redirect(request_target, raw_pathname) {
        return write_location_response(stream, "308 Permanent Redirect", &location);
    }
    let raw_query = request_target.split_once('?').map(|(_, query)| query);
    let mut search_params = match parse_search_params(raw_query) {
        Some(search_params) => search_params,
        None => {
            return write_response(
                stream,
                "400 Bad Request",
                "text/plain",
                b"Invalid query string",
            );
        }
    };
    // `_rsc` is an App Router transport cache-buster, not user search state.
    search_params.remove("_rsc");
    let rendered_search = ordered_search(raw_query).ok_or("Invalid query string")?;
    if method == "POST" && has_native_mutation_route(&pathname) {
        return execute_native_revalidation(stream, &headers, &pathname);
    }
    if method != "GET" && method != "HEAD" {
        return write_method_not_allowed(stream);
    }
    if request_kind == RequestKind::SegmentPrefetch {
        let Some(request_key) = segment_prefetch_key else {
            return write_response(
                stream,
                "400 Bad Request",
                "text/plain",
                b"Missing segment key",
            );
        };
        if request_key == "/_tree" {
            let Some(route_pattern) = native_ppr_route_pattern(pathname) else {
                return fallback_segment_prefetch(
                    stream,
                    &request,
                    headers_end,
                    "PPR tree is unsupported for this native route",
                );
            };
            return write_native_ppr_tree(stream, route_pattern, pathname, method);
        }
        let segment = if pathname == "/catalog/rust" && request_key == "/catalog/rust/__PAGE__" {
            Some(catalog::render(
                &search_params,
                Arc::new(AtomicBool::new(false)),
            ))
        } else {
            render_native_ppr_segment(pathname, request_key, &search_params, &request_data)
        };
        let Some(segment) = segment else {
            return fallback_segment_prefetch(
                stream,
                &request,
                headers_end,
                "PPR segment is unsupported for this native route",
            );
        };
        return match segment {
            Ok(node) => {
                write_native_ppr_segment(stream, node, method, request_data.accessed_runtime_data())
            }
            Err(error) => write_render_error(
                stream,
                error,
                true,
                method,
                request_data.accessed_runtime_data(),
            ),
        };
    }
    if request_kind == RequestKind::InterceptionNavigation {
        if let Ok(fallback) = env::var("NEXT_FALLBACK_ADDR") {
            trace_request_selection(request_id, method, request_target, request_kind, "fallback");
            return proxy_to_fallback(stream, &fallback, &request, headers_end);
        }
        return write_response(
            stream,
            "501 Not Implemented",
            "text/plain",
            b"Native interception navigation requires the Next.js fallback",
        );
    }
    if pathname == "/catalog/rust" && request_kind == RequestKind::Document && method == "GET" {
        trace_request_selection(request_id, method, request_target, request_kind, "native");
        return write_streaming_catalog_document(stream, &search_params, &rendered_search);
    }
    if pathname == "/catalog/rust" && request_kind != RequestKind::Document && method == "GET" {
        trace_request_selection(request_id, method, request_target, request_kind, "native");
        if request_kind == RequestKind::NavigationRefetch {
            let cancelled = Arc::new(AtomicBool::new(false));
            let tree = catalog::render(&search_params, cancelled)?;
            return write_flight_response(
                stream,
                &catalog_refetch_payload(&rendered_search, tree),
                method,
                true,
            );
        }
        return write_streaming_catalog_flight(stream, &search_params, &rendered_search);
    }
    if let Some(id) = pathname.strip_prefix("/catalog/rust/product/")
        && request_kind != RequestKind::Document
        && method == "GET"
    {
        let Some(page) = render_native_ppr_segment(
            pathname,
            "/catalog/rust/product/$d$id/__PAGE__",
            &search_params,
            &request_data,
        ) else {
            return write_response(stream, "404 Not Found", "text/plain", b"Not found");
        };
        let page = page?;
        let page = next_rsc::client_reference(
            RUST_RSC_CATALOG_DETAIL_CONTROLS_MODULE_ID,
            "default",
            RUST_RSC_CATALOG_DETAIL_CONTROLS_CHUNKS.iter().copied(),
            [page],
        )
        .prop("basePath", "/catalog/rust");
        trace_request_selection(request_id, method, request_target, request_kind, "native");
        return write_flight_response(
            stream,
            &catalog_product_navigation_payload(id, page),
            method,
            request_data.accessed_runtime_data(),
        );
    }
    let rendered = if pathname == "/catalog/rust" {
        let cancelled = Arc::new(AtomicBool::new(false));
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let monitor =
            monitor_disconnect(stream, Arc::clone(&cancelled), Arc::clone(&monitor_stop))?;
        let result = catalog::render(&search_params, cancelled).and_then(|node| {
            component_0::render(next_rsc::LayoutProps::new(
                node,
                next_rsc::Params::default(),
            ))
        });
        monitor_stop.store(true, Ordering::Release);
        let _ = monitor.join();
        Some(result)
    } else {
        render_native_path(pathname, &search_params, &request_data)
    };
    let Some(tree) = rendered else {
        if let Ok(fallback) = env::var("NEXT_FALLBACK_ADDR") {
            trace_request_selection(request_id, method, request_target, request_kind, "fallback");
            return proxy_to_fallback(stream, &fallback, &request, headers_end);
        }
        return write_response(stream, "404 Not Found", "text/plain", b"Not found");
    };
    trace_request_selection(request_id, method, request_target, request_kind, "native");
    let wants_flight = request_kind != RequestKind::Document;
    let tree = match tree {
        Ok(tree) => tree,
        Err(error) => {
            return write_render_error(
                stream,
                error,
                wants_flight,
                method,
                request_data.accessed_runtime_data(),
            );
        }
    };

    if wants_flight {
        write_flight_response(
            stream,
            &navigation_payload(pathname, &rendered_search, tree),
            method,
            request_data.accessed_runtime_data(),
        )
    } else {
        let mut html = String::from("<!DOCTYPE html>");
        push_html(&mut html, &tree)?;
        let flight = next_rsc_flight::encode_root_value(&navigation_payload(
            pathname,
            &rendered_search,
            tree,
        ))?;
        inject_app_router_bootstrap(&mut html, &flight)?;
        write_response_for_method_with_cache(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            html.as_bytes(),
            method,
            request_data.accessed_runtime_data(),
        )
    }
}

fn execute_native_revalidation(
    stream: &mut TcpStream,
    headers: &[(&str, &str)],
    pathname: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use next_rsc::{RevalidationPathKind, RevalidationRequest};
    let Ok(expected_token) = env::var("RUST_RSC_REVALIDATE_TOKEN") else {
        return write_response_for_method_with_cache(
            stream,
            "503 Service Unavailable",
            "application/json",
            br#"{"error":"native revalidation is disabled"}"#,
            "POST",
            true,
        );
    };
    let provided_token = headers.iter().find_map(|(name, value)| {
        if name.eq_ignore_ascii_case("authorization") {
            value.trim().strip_prefix("Bearer ")
        } else if name.eq_ignore_ascii_case("x-rust-rsc-revalidate-token") {
            Some(value.trim())
        } else {
            None
        }
    });
    if expected_token.is_empty()
        || !provided_token.is_some_and(|provided| constant_time_equal(provided, &expected_token))
    {
        return write_response_for_method_with_cache(
            stream,
            "401 Unauthorized",
            "application/json",
            br#"{"error":"unauthorized"}"#,
            "POST",
            true,
        );
    }

    let mut context = next_rsc::MutationContext::default();
    execute_native_mutation_route(pathname, &mut context)
        .ok_or("native mutation route disappeared")??;
    let requests = context.into_requests();
    if requests.iter().any(|request| {
        !matches!(
            request,
            RevalidationRequest::Tag { tag, profile }
                if tag == "catalog" && profile == "max"
        ) && !matches!(request, RevalidationRequest::UpdateTag { tag } if tag == "catalog")
            && !matches!(
                request,
                RevalidationRequest::Path {
                    path,
                    kind: Some(RevalidationPathKind::Page),
                } if path == "/catalog/rust"
            )
    }) {
        return write_response_for_method_with_cache(
            stream,
            "501 Not Implemented",
            "application/json",
            br#"{"error":"unsupported native revalidation request"}"#,
            "POST",
            true,
        );
    }

    let mut cleared = 0;
    let mut applied = 0;
    for request in &requests {
        match request {
            RevalidationRequest::Tag { tag, .. } | RevalidationRequest::UpdateTag { tag }
                if tag == "catalog" =>
            {
                cleared += catalog::invalidate_warm_cache();
                applied += 1;
            }
            RevalidationRequest::Path { path, .. } if path == "/catalog/rust" => {
                cleared += catalog::invalidate_warm_cache();
                applied += 1;
            }
            _ => unreachable!("unsupported revalidation was rejected before mutation"),
        }
    }
    let body = format!("{{\"applied\":{applied},\"warmEntriesCleared\":{cleared}}}");
    write_response_for_method_with_cache(
        stream,
        "200 OK",
        "application/json",
        body.as_bytes(),
        "POST",
        true,
    )
}

fn constant_time_equal(provided: &str, expected: &str) -> bool {
    let provided = provided.as_bytes();
    let expected = expected.as_bytes();
    let mut difference = provided.len() ^ expected.len();
    for index in 0..provided.len().max(expected.len()) {
        difference |= usize::from(
            provided.get(index).copied().unwrap_or(0) ^ expected.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

fn fallback_segment_prefetch(
    stream: &mut TcpStream,
    request: &[u8],
    headers_end: usize,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(fallback) = env::var("NEXT_FALLBACK_ADDR") {
        proxy_to_fallback(stream, &fallback, request, headers_end)
    } else {
        write_response(
            stream,
            "501 Not Implemented",
            "text/plain",
            message.as_bytes(),
        )
    }
}

fn ppr_tree_node(
    segment: &str,
    child: Option<next_rsc_flight::FlightValue>,
    mut extra_slots: BTreeMap<String, next_rsc_flight::FlightValue>,
) -> next_rsc_flight::FlightValue {
    use next_rsc_flight::FlightValue::{Null, Number, Object, String as FlightString};
    let (name, param) = if segment.starts_with("[[...") && segment.ends_with("]]") {
        (&segment[5..segment.len() - 2], Some("oc"))
    } else if segment.starts_with("[...") && segment.ends_with(']') {
        (&segment[4..segment.len() - 1], Some("c"))
    } else if segment.starts_with('[') && segment.ends_with(']') {
        (&segment[1..segment.len() - 1], Some("d"))
    } else {
        (segment, None)
    };
    let param = param.map_or(Null, |kind| {
        next_rsc_flight::FlightValue::object([
            ("type", FlightString(kind.to_owned())),
            ("key", Null),
            ("siblings", Null),
        ])
    });
    if let Some(value) = child {
        extra_slots.insert("children".to_owned(), value);
    }
    next_rsc_flight::FlightValue::object([
        ("name", FlightString(name.to_owned())),
        ("param", param),
        ("prefetchHints", Number(0.0)),
        (
            "slots",
            if extra_slots.is_empty() {
                Null
            } else {
                Object(extra_slots)
            },
        ),
    ])
}

fn write_native_ppr_tree(
    stream: &mut TcpStream,
    route_pattern: &str,
    pathname: &str,
    method: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use next_rsc_flight::FlightValue::{Number, String as FlightString};
    let pattern_parts: Vec<_> = route_pattern
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let pathname_parts: Vec<_> = pathname
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let mut child = ppr_tree_node("__PAGE__", None, BTreeMap::new());
    for (index, segment) in pattern_parts.iter().enumerate().rev() {
        let mount = if index < pathname_parts.len() {
            format!("/{}", pathname_parts[..=index].join("/"))
        } else {
            pathname.to_owned()
        };
        let mut slots = BTreeMap::new();
        for (slot_mount, slot_name, suffix) in native_parallel_slot_paths(pathname) {
            if *slot_mount == mount {
                let mut slot = ppr_tree_node("__PAGE__", None, BTreeMap::new());
                for slot_segment in suffix.split('/').filter(|part| !part.is_empty()).rev() {
                    slot = ppr_tree_node(slot_segment, Some(slot), BTreeMap::new());
                }
                slots.insert((*slot_name).to_owned(), slot);
            }
        }
        child = ppr_tree_node(segment, Some(child), slots);
    }
    let tree = ppr_tree_node("", Some(child), BTreeMap::new());
    let payload = next_rsc_flight::FlightValue::object([
        ("buildId", FlightString(RUST_RSC_BUILD_ID.to_owned())),
        ("tree", tree),
        ("staleTime", Number(300.0)),
    ]);
    write_ppr_flight_response(stream, &payload, method, false)
}

fn write_native_ppr_segment(
    stream: &mut TcpStream,
    node: Node,
    method: &str,
    runtime_data_accessed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use next_rsc_flight::FlightValue::{
        Array, AsyncIterable, Bool, Deferred, Node as FlightNode, Null, Number,
        String as FlightString,
    };
    let stale_time = if runtime_data_accessed { 0.0 } else { 300.0 };
    let segment = next_rsc_flight::FlightValue::object([
        ("rsc", FlightNode(node)),
        ("isPartial", Deferred(Box::new(Null))),
        (
            "staleTime",
            AsyncIterable {
                iterator: false,
                values: vec![Number(stale_time)],
                completion: None,
            },
        ),
        ("varyParams", Null),
    ]);
    let payload = next_rsc_flight::FlightValue::object([
        ("buildId", FlightString(RUST_RSC_BUILD_ID.to_owned())),
        ("data", Array(vec![segment])),
        ("isUpgradeableISRFallback", Bool(false)),
        ("a", Deferred(Box::new(Null))),
        ("rootVaryParams", Null),
        ("needsRuntimeRequest", Deferred(Box::new(Bool(false)))),
    ]);
    write_ppr_flight_response(stream, &payload, method, runtime_data_accessed)
}

fn write_ppr_flight_response(
    stream: &mut TcpStream,
    payload: &next_rsc_flight::FlightValue,
    method: &str,
    runtime_data_accessed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let body = next_rsc_flight::encode_root_value(payload)?;
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/x-component\r\nx-nextjs-postponed: \
         2\r\nContent-Length: {}\r\nCache-Control: {}\r\nVary: RSC, Next-Router-State-Tree, \
         Next-Router-Prefetch, Next-Router-Segment-Prefetch\r\nX-Content-Type-Options: \
         nosniff\r\nConnection: close\r\n\r\n",
        body.len(),
        route_cache_control(runtime_data_accessed),
    );
    stream.write_all(headers.as_bytes())?;
    if method != "HEAD" {
        stream.write_all(&body)?;
    }
    Ok(())
}

fn trace_request_selection(
    request_id: usize,
    method: &str,
    target: &str,
    kind: RequestKind,
    selected: &str,
) {
    if env::var_os("RUST_RSC_TRACE").is_some() {
        eprintln!(
            "rust_rsc request_select id={request_id} method={method} target={target:?} \
             kind={kind:?} selected={selected} compression=identity"
        );
    }
}

#[derive(Debug, Eq, PartialEq)]
enum NativeRewrite {
    None,
    Internal(String),
    Fallback,
}

fn apply_native_rewrite(pathname: &str) -> NativeRewrite {
    for (source, destination, fallback_only) in RUST_RSC_REWRITES {
        if let Some(destination) = match_rewrite(source, destination, pathname) {
            return if *fallback_only {
                NativeRewrite::Fallback
            } else {
                NativeRewrite::Internal(destination)
            };
        }
    }
    NativeRewrite::None
}

fn match_rewrite(source: &str, destination: &str, pathname: &str) -> Option<String> {
    let source_segments: Vec<_> = source.trim_matches('/').split('/').collect();
    let path_segments: Vec<_> = pathname.trim_matches('/').split('/').collect();
    let mut values = BTreeMap::new();
    let mut path_index = 0;
    for (index, source_segment) in source_segments.iter().enumerate() {
        if let Some(parameter) = source_segment.strip_prefix(':') {
            let (name, modifier) = match parameter.as_bytes().last() {
                Some(b'*') | Some(b'+') => (
                    &parameter[..parameter.len() - 1],
                    parameter.as_bytes()[parameter.len() - 1],
                ),
                _ => (parameter, b'\0'),
            };
            if modifier == b'*' || modifier == b'+' {
                let remaining = path_segments[path_index..].join("/");
                if modifier == b'+' && remaining.is_empty() {
                    return None;
                }
                values.insert(name, remaining);
                path_index = path_segments.len();
                if index + 1 != source_segments.len() {
                    return None;
                }
            } else {
                values.insert(name, (*path_segments.get(path_index)?).to_owned());
                path_index += 1;
            }
        } else {
            if path_segments.get(path_index) != Some(source_segment) {
                return None;
            }
            path_index += 1;
        }
    }
    if path_index != path_segments.len() {
        return None;
    }
    let mut output = Vec::new();
    for segment in destination.trim_matches('/').split('/') {
        if let Some(parameter) = segment.strip_prefix(':') {
            let name = parameter.trim_end_matches(['*', '+']);
            output.push(values.get(name)?.as_str());
        } else {
            output.push(segment);
        }
    }
    Some(format!("/{}", output.join("/")))
}

fn normalize_request_path(pathname: &str) -> Option<String> {
    let mut pathname = if RUST_RSC_BASE_PATH.is_empty() {
        pathname
    } else if pathname == RUST_RSC_BASE_PATH {
        "/"
    } else {
        pathname
            .strip_prefix(RUST_RSC_BASE_PATH)?
            .strip_prefix('/')
            .map(|path| {
                // Retain the leading slash expected by the generated matcher.
                path
            })?
    };
    let owned;
    if !RUST_RSC_BASE_PATH.is_empty() && pathname != "/" {
        owned = format!("/{pathname}");
        pathname = &owned;
    }
    let mut normalized = pathname.to_owned();
    if normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    let first = normalized.split('/').find(|segment| !segment.is_empty());
    if let Some(locale) = first.filter(|segment| RUST_RSC_LOCALES.contains(segment)) {
        normalized = normalized
            .strip_prefix(&format!("/{locale}"))
            .unwrap_or(&normalized)
            .to_owned();
        if normalized.is_empty() {
            normalized.push('/');
        }
    }
    Some(normalized)
}

fn trailing_slash_redirect(request_target: &str, pathname: &str) -> Option<String> {
    let base_root = !RUST_RSC_BASE_PATH.is_empty() && pathname == RUST_RSC_BASE_PATH;
    let has_extension = pathname
        .rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'));
    let needs_slash =
        RUST_RSC_TRAILING_SLASH && !pathname.ends_with('/') && !has_extension && pathname != "/";
    let removes_slash =
        !RUST_RSC_TRAILING_SLASH && pathname.ends_with('/') && pathname != "/" && !base_root;
    if !needs_slash && !removes_slash {
        return None;
    }
    let (path, query) = request_target
        .split_once('?')
        .map_or((request_target, None), |(path, query)| (path, Some(query)));
    let mut location = if needs_slash {
        format!("{path}/")
    } else {
        path.trim_end_matches('/').to_owned()
    };
    if let Some(query) = query {
        write!(location, "?{query}").unwrap();
    }
    Some(location)
}

fn write_location_response(
    stream: &mut TcpStream,
    status: &str,
    location: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = format!(
        "HTTP/1.1 {status}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: \
         close\r\n\r\n"
    );
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn write_streaming_catalog_document(
    stream: &mut TcpStream,
    search_params: &BTreeMap<String, next_rsc::ParamValue>,
    rendered_search: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nTransfer-Encoding: chunked\r\nCache-Control: private, no-store\r\nVary: RSC, Next-Router-State-Tree, Next-Router-Prefetch, Next-Router-Segment-Prefetch\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n")?;
    let mut initial = format!(
        "<!DOCTYPE html><html><head><script>self.__next_r=\"{RUST_RSC_BUILD_ID}\"</script><link \
         rel=\"preload\" as=\"script\" fetchpriority=\"low\" href=\"{RUST_RSC_WEBPACK_ASSET}\">"
    );
    push_native_metadata(&mut initial);
    for asset in RUST_RSC_MAIN_ASSETS {
        write!(initial, "<script src=\"{asset}\" async></script>").unwrap();
    }
    for asset in RUST_RSC_ENTRY_CLIENT_ASSETS {
        write!(initial, "<script src=\"{asset}\" async></script>").unwrap();
    }
    for asset in RUST_RSC_POLYFILL_ASSETS {
        write!(initial, "<script src=\"{asset}\" nomodule></script>").unwrap();
    }
    for asset in RUST_RSC_CSS_ASSETS {
        write!(initial, "<link rel=\"stylesheet\" href=\"{asset}\">").unwrap();
    }
    initial.push_str(
        "</head><body><main data-renderer=\"rust\">Rendered by Rust (hot edit)<div \
         class=\"catalog-shell\" data-catalog=\"rust\"><header class=\"catalog-header\"><a \
         class=\"catalog-brand\" href=\"/catalog/rust\">RustWorks Supply</a><form \
         class=\"catalog-search\"><label for=\"catalog-q\">Search products</label><input \
         id=\"catalog-q\" name=\"q\" value=\"",
    );
    if let Some(value) = search_params.get("q") {
        let value = match value {
            next_rsc::ParamValue::String(value) => value.as_str(),
            next_rsc::ParamValue::Strings(values) => values.first().map_or("", String::as_str),
        };
        push_html_attribute_escaped(&mut initial, value);
    }
    initial.push_str(
        "\"><button>Search</button></form></header><div class=\"catalog-grid\"><nav \
         class=\"catalog-sidebar\" aria-label=\"Categories\" \
         data-loading-region=\"categories\">Loading categories...</nav><section \
         aria-label=\"Products\" data-loading-region=\"products\">Loading \
         products...</section></div></div>",
    );
    // PROTOTYPE: this prefix mirrors app/layout.rs so useful catalog shell and
    // two independent loading regions reach the browser before data resolves.
    write_http_chunk(stream, initial.as_bytes())?;
    stream.flush()?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor = monitor_disconnect(stream, Arc::clone(&cancelled), Arc::clone(&monitor_stop))?;
    let mut on_region = |name: &str, node: &Node| {
        let mut chunk = format!("<div data-rust-rsc-resolution=\"{name}\" hidden>");
        push_html(&mut chunk, node).map_err(RenderError::new)?;
        write!(
            chunk,
            "</div><script>document.querySelector('[data-loading-region=\"{name}\"]').\
             replaceWith(document.querySelector('[data-rust-rsc-resolution=\"{name}\"]').\
             firstElementChild);document.querySelector('[data-rust-rsc-resolution=\"{name}\"]').\
             remove();document.currentScript.remove()</script>"
        )
        .unwrap();
        write_http_chunk(stream, chunk.as_bytes())
            .map_err(|error| RenderError::new(error.to_string()))?;
        stream
            .flush()
            .map_err(|error| RenderError::new(error.to_string()))?;
        Ok(())
    };
    let page = catalog::render_streaming(search_params, cancelled, &mut on_region)?;
    monitor_stop.store(true, Ordering::Release);
    let _ = monitor.join();
    let mut resolved = String::from("</main>");
    let flight =
        next_rsc_flight::encode_root_value(&catalog_initial_payload(rendered_search, page)?)?;
    push_app_router_body_scripts(&mut resolved, std::str::from_utf8(&flight)?);
    resolved.push_str("</body></html>");
    write_http_chunk(stream, resolved.as_bytes())?;
    stream.write_all(b"0\r\n\r\n")?;
    Ok(())
}

fn write_streaming_catalog_flight(
    stream: &mut TcpStream,
    search_params: &BTreeMap<String, next_rsc::ParamValue>,
    rendered_search: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/x-component\r\nTransfer-Encoding: chunked\r\nCache-Control: private, no-store\r\nVary: RSC, Next-Router-State-Tree, Next-Router-Prefetch, Next-Router-Segment-Prefetch\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n")?;
    let mut graph = next_rsc_flight::FlightTaskGraph::new(Default::default());
    let (task_id, pending_seed) = graph.reserve_task()?;
    let pending = catalog_navigation_payload_with_seed(rendered_search, pending_seed);
    graph.enqueue_root(&pending)?;
    graph.drain_to_sink(&mut HttpChunkSink(stream))?;
    stream.flush()?;

    let cancelled = Arc::new(AtomicBool::new(false));
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor = monitor_disconnect(stream, Arc::clone(&cancelled), Arc::clone(&monitor_stop))?;
    let rendered = catalog::render(search_params, cancelled);
    monitor_stop.store(true, Ordering::Release);
    let _ = monitor.join();
    match rendered {
        Ok(tree) => {
            graph.resolve_task(task_id, &next_rsc_flight::FlightValue::Node(tree))?;
        }
        Err(error) => graph.error_task(task_id, &error.flight_digest())?,
    }
    graph.drain_to_sink(&mut HttpChunkSink(stream))?;
    stream.write_all(b"0\r\n\r\n")?;
    Ok(())
}

struct HttpChunkSink<'a>(&'a mut TcpStream);

impl next_rsc_flight::FlightSink for HttpChunkSink<'_> {
    type Error = std::io::Error;

    fn write_chunk(&mut self, chunk: &[u8]) -> Result<next_rsc_flight::SinkStatus, Self::Error> {
        write_http_chunk(self.0, chunk)?;
        Ok(next_rsc_flight::SinkStatus::Ready)
    }
}

fn catalog_navigation_payload_with_seed(
    rendered_search: &str,
    seed_node: next_rsc_flight::FlightValue,
) -> next_rsc_flight::FlightValue {
    use next_rsc_flight::FlightValue::{
        Array, Bool, Null, Number, Object, String as FlightString, Undefined,
    };
    let page_segment = format!("__PAGE__?{}", search_params_json(rendered_search));
    let tree_patch = Array(vec![
        FlightString(page_segment.clone()),
        Object(Default::default()),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    let seed_data = Array(vec![
        seed_node,
        Object(Default::default()),
        Null,
        Bool(false),
        Null,
    ]);
    let flight_path = Array(vec![
        FlightString("children".to_owned()),
        FlightString("catalog".to_owned()),
        FlightString("children".to_owned()),
        FlightString("rust".to_owned()),
        FlightString("children".to_owned()),
        FlightString(page_segment),
        tree_patch,
        seed_data,
        Null,
        Bool(false),
    ]);
    next_rsc_flight::FlightValue::object([
        (
            "c",
            Array(vec![
                FlightString("".to_owned()),
                FlightString("catalog".to_owned()),
                FlightString("rust".to_owned()),
            ]),
        ),
        ("f", Array(vec![flight_path])),
        ("q", FlightString(rendered_search.to_owned())),
        ("i", Bool(false)),
        ("h", Null),
        ("r", Undefined),
        ("G", Array(vec![Null, Undefined])),
        ("S", Bool(false)),
        ("b", FlightString(RUST_RSC_BUILD_ID.to_owned())),
    ])
}

fn catalog_product_navigation_payload(id: &str, page_node: Node) -> next_rsc_flight::FlightValue {
    use next_rsc_flight::FlightValue::{
        Array, Bool, Node as FlightNode, Null, Number, Object, String as FlightString, Undefined,
    };
    let page_tree = Array(vec![
        FlightString("__PAGE__".to_owned()),
        Object(Default::default()),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    let id_tree = Array(vec![
        Array(vec![
            FlightString("id".to_owned()),
            FlightString(id.to_owned()),
            FlightString("d".to_owned()),
            Array(Vec::new()),
        ]),
        Object(BTreeMap::from([("children".to_owned(), page_tree)])),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    let product_tree = Array(vec![
        FlightString("product".to_owned()),
        Object(BTreeMap::from([("children".to_owned(), id_tree)])),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    let page_seed = Array(vec![
        FlightNode(page_node),
        Object(Default::default()),
        Null,
        Bool(false),
        Null,
    ]);
    let id_seed = Array(vec![
        FlightNode(router_fragment()),
        Object(BTreeMap::from([("children".to_owned(), page_seed)])),
        Null,
        Bool(false),
        Null,
    ]);
    let product_seed = Array(vec![
        FlightNode(router_fragment()),
        Object(BTreeMap::from([("children".to_owned(), id_seed)])),
        Null,
        Bool(false),
        Null,
    ]);
    let flight_path = Array(vec![
        FlightString("children".to_owned()),
        FlightString("catalog".to_owned()),
        FlightString("children".to_owned()),
        FlightString("rust".to_owned()),
        FlightString("children".to_owned()),
        FlightString("product".to_owned()),
        product_tree,
        product_seed,
        Null,
        Bool(false),
    ]);
    next_rsc_flight::FlightValue::object([
        (
            "c",
            Array(vec![
                FlightString("".to_owned()),
                FlightString("catalog".to_owned()),
                FlightString("rust".to_owned()),
                FlightString("product".to_owned()),
                FlightString(id.to_owned()),
            ]),
        ),
        ("f", Array(vec![flight_path])),
        ("q", FlightString(String::new())),
        ("i", Bool(false)),
        ("h", Null),
        ("r", Undefined),
        ("G", Array(vec![Null, Undefined])),
        ("S", Bool(false)),
        ("b", FlightString(RUST_RSC_BUILD_ID.to_owned())),
    ])
}

fn catalog_refetch_payload(rendered_search: &str, page_node: Node) -> next_rsc_flight::FlightValue {
    use next_rsc_flight::FlightValue::{
        Array, Bool, Node as FlightNode, Null, Number, Object, String as FlightString, Undefined,
    };
    let page_tree = Array(vec![
        FlightString(format!("__PAGE__?{}", search_params_json(rendered_search))),
        Object(Default::default()),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    let rust_tree = Array(vec![
        FlightString("rust".to_owned()),
        Object(BTreeMap::from([("children".to_owned(), page_tree)])),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    let page_seed = Array(vec![
        FlightNode(Node::react_fragment(Some("c"), [page_node])),
        Object(Default::default()),
        Null,
        Bool(false),
        Null,
    ]);
    let rust_seed = Array(vec![
        FlightNode(router_fragment()),
        Object(BTreeMap::from([("children".to_owned(), page_seed)])),
        Null,
        Bool(false),
        Null,
    ]);
    let catalog_seed = Array(vec![
        FlightNode(router_fragment()),
        Object(BTreeMap::from([("children".to_owned(), rust_seed)])),
        Null,
        Bool(false),
        Null,
    ]);
    next_rsc_flight::FlightValue::object([
        (
            "f",
            Array(vec![Array(vec![
                FlightString("children".to_owned()),
                FlightString("catalog".to_owned()),
                Array(vec![
                    FlightString("catalog".to_owned()),
                    Object(BTreeMap::from([("children".to_owned(), rust_tree)])),
                    Undefined,
                    Undefined,
                    Number(4096.0),
                ]),
                catalog_seed,
                Null,
                Bool(false),
            ])]),
        ),
        ("q", FlightString(rendered_search.to_owned())),
        ("i", Bool(false)),
        ("h", Null),
        ("r", Undefined),
        ("G", Array(vec![Null, Undefined])),
        ("S", Bool(false)),
        ("b", FlightString(RUST_RSC_BUILD_ID.to_owned())),
    ])
}

fn next_layout_router(parallel_router_key: &str) -> Node {
    let template = next_rsc::client_reference(
        RUST_RSC_TEMPLATE_CONTEXT_MODULE_ID,
        RUST_RSC_TEMPLATE_CONTEXT_EXPORT,
        RUST_RSC_TEMPLATE_CONTEXT_CHUNKS.iter().copied(),
        [],
    );
    next_rsc::client_reference(
        RUST_RSC_LAYOUT_ROUTER_MODULE_ID,
        RUST_RSC_LAYOUT_ROUTER_EXPORT,
        RUST_RSC_LAYOUT_ROUTER_CHUNKS.iter().copied(),
        [],
    )
    .prop("parallelRouterKey", parallel_router_key)
    .prop("error", PropValue::Undefined)
    .prop("errorStyles", PropValue::Undefined)
    .prop("errorScripts", PropValue::Undefined)
    .prop("template", PropValue::Node(Box::new(template)))
    .prop("templateStyles", PropValue::Undefined)
    .prop("templateScripts", PropValue::Undefined)
    .prop("notFound", PropValue::Undefined)
    .prop("forbidden", PropValue::Undefined)
    .prop("unauthorized", PropValue::Undefined)
}

fn router_fragment() -> Node {
    Node::react_fragment(Some("c"), [Node::Null, next_layout_router("children")])
}

fn catalog_initial_payload(
    rendered_search: &str,
    page_node: Node,
) -> Result<next_rsc_flight::FlightValue, next_rsc::RenderError> {
    use next_rsc_flight::FlightValue::{
        Array, Bool, Node as FlightNode, Null, Number, Object, String as FlightString, Undefined,
    };
    let page_segment = if rendered_search.is_empty() {
        "__PAGE__".to_owned()
    } else {
        format!("__PAGE__?{}", search_params_json(rendered_search))
    };
    let page_tree = Array(vec![
        FlightString(page_segment),
        Object(Default::default()),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    let rust_tree = Array(vec![
        FlightString("rust".to_owned()),
        Object(BTreeMap::from([("children".to_owned(), page_tree)])),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    let catalog_tree = Array(vec![
        FlightString("catalog".to_owned()),
        Object(BTreeMap::from([("children".to_owned(), rust_tree)])),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    let root_tree = Array(vec![
        FlightString("".to_owned()),
        Object(BTreeMap::from([("children".to_owned(), catalog_tree)])),
        Null,
        Null,
        Number(4112.0),
    ]);
    let page_seed = Array(vec![
        FlightNode(Node::react_fragment(Some("c"), [page_node])),
        Object(Default::default()),
        Null,
        Bool(false),
        Null,
    ]);
    let rust_seed = Array(vec![
        FlightNode(router_fragment()),
        Object(BTreeMap::from([("children".to_owned(), page_seed)])),
        Null,
        Bool(false),
        Null,
    ]);
    let catalog_seed = Array(vec![
        FlightNode(router_fragment()),
        Object(BTreeMap::from([("children".to_owned(), rust_seed)])),
        Null,
        Bool(false),
        Null,
    ]);
    let root_node = component_0::render(next_rsc::LayoutProps::new(
        next_layout_router("children"),
        next_rsc::Params::default(),
    ))?;
    let root_seed = Array(vec![
        FlightNode(root_node),
        Object(BTreeMap::from([("children".to_owned(), catalog_seed)])),
        Null,
        Bool(false),
        Null,
    ]);
    Ok(next_rsc_flight::FlightValue::object([
        (
            "c",
            Array(vec![
                FlightString("".to_owned()),
                FlightString("catalog".to_owned()),
                FlightString("rust".to_owned()),
            ]),
        ),
        (
            "f",
            Array(vec![Array(vec![root_tree, root_seed, Null, Bool(false)])]),
        ),
        ("q", FlightString(rendered_search.to_owned())),
        ("i", Bool(false)),
        ("h", Null),
        ("r", Undefined),
        ("G", Array(vec![Null, Undefined])),
        ("S", Bool(false)),
        ("b", FlightString(RUST_RSC_BUILD_ID.to_owned())),
    ]))
}

fn write_http_chunk(stream: &mut TcpStream, bytes: &[u8]) -> std::io::Result<()> {
    write!(stream, "{:x}\r\n", bytes.len())?;
    stream.write_all(bytes)?;
    stream.write_all(b"\r\n")
}

fn monitor_disconnect(
    stream: &TcpStream,
    cancelled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, std::io::Error> {
    let probe = stream.try_clone()?;
    probe.set_read_timeout(Some(Duration::from_millis(2)))?;
    Ok(thread::spawn(move || {
        let mut byte = [0_u8; 1];
        while !stop.load(Ordering::Acquire) {
            match probe.peek(&mut byte) {
                Ok(0) => {
                    cancelled.store(true, Ordering::Release);
                    break;
                }
                Ok(_) => thread::sleep(Duration::from_millis(5)),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => {
                    cancelled.store(true, Ordering::Release);
                    break;
                }
            }
        }
    }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestKind {
    Document,
    Navigation,
    NavigationRefetch,
    Prefetch,
    SegmentPrefetch,
    InterceptionNavigation,
}

fn classify_request(headers: &[(&str, &str)]) -> Result<RequestKind, String> {
    let header = |wanted: &str| {
        headers
            .iter()
            .find(|(name, _)| name.trim().eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.trim())
    };
    if header("rsc") != Some("1") {
        return Ok(RequestKind::Document);
    }
    if let Some(state) = header("next-router-state-tree") {
        let decoded = percent_decode_path_segment(state)
            .ok_or_else(|| "Malformed Next-Router-State-Tree header".to_owned())?;
        let state = serde_json::from_str::<serde_json::Value>(&decoded)
            .map_err(|_| "Malformed Next-Router-State-Tree header".to_owned())?;
        if !state.is_array() {
            return Err("Malformed Next-Router-State-Tree header".to_owned());
        }
        if contains_interception_marker(&state) {
            return Ok(RequestKind::InterceptionNavigation);
        }
        if decoded.contains("\"refetch\"") {
            return Ok(RequestKind::NavigationRefetch);
        }
    }
    if header("next-router-segment-prefetch").is_some() {
        Ok(RequestKind::SegmentPrefetch)
    } else if header("next-router-prefetch") == Some("1") {
        Ok(RequestKind::Prefetch)
    } else {
        Ok(RequestKind::Navigation)
    }
}

fn contains_interception_marker(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(value) => {
            value.starts_with("(.)") || value.starts_with("(..)") || value.starts_with("(...)")
        }
        serde_json::Value::Array(values) => values.iter().any(contains_interception_marker),
        serde_json::Value::Object(values) => values.values().any(contains_interception_marker),
        _ => false,
    }
}

fn parse_search_params(query: Option<&str>) -> Option<BTreeMap<String, next_rsc::ParamValue>> {
    let mut result = BTreeMap::new();
    for pair in query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
    {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let name = percent_decode_path_segment(&name.replace('+', " "))?;
        let value = percent_decode_path_segment(&value.replace('+', " "))?;
        match result.remove(&name) {
            None => {
                result.insert(name, next_rsc::ParamValue::String(value));
            }
            Some(next_rsc::ParamValue::String(previous)) => {
                result.insert(name, next_rsc::ParamValue::Strings(vec![previous, value]));
            }
            Some(next_rsc::ParamValue::Strings(mut values)) => {
                values.push(value);
                result.insert(name, next_rsc::ParamValue::Strings(values));
            }
        }
    }
    Some(result)
}

fn ordered_search(query: Option<&str>) -> Option<String> {
    let mut output = String::new();
    for pair in query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
    {
        let (raw_name, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let name = percent_decode_path_segment(&raw_name.replace('+', " "))?;
        if name == "_rsc" {
            continue;
        }
        let value = percent_decode_path_segment(&raw_value.replace('+', " "))?;
        output.push(if output.is_empty() { '?' } else { '&' });
        push_query_component(&mut output, &name);
        output.push('=');
        push_query_component(&mut output, &value);
    }
    Some(output)
}

fn search_params_json(rendered_search: &str) -> String {
    let mut entries: Vec<(String, Vec<String>)> = Vec::new();
    for pair in rendered_search
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
    {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let name = percent_decode_path_segment(name).unwrap_or_default();
        let value = percent_decode_path_segment(value).unwrap_or_default();
        if let Some((_, values)) = entries.iter_mut().find(|(existing, _)| existing == &name) {
            values.push(value);
        } else {
            entries.push((name, vec![value]));
        }
    }
    let mut output = String::from("{");
    for (index, (name, values)) in entries.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&serde_json::to_string(name).unwrap());
        output.push(':');
        if values.len() == 1 {
            output.push_str(&serde_json::to_string(&values[0]).unwrap());
        } else {
            output.push_str(&serde_json::to_string(values).unwrap());
        }
    }
    output.push('}');
    output
}

fn push_query_component(output: &mut String, value: &str) {
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(byte));
        } else {
            write!(output, "%{byte:02X}").unwrap();
        }
    }
}

fn write_flight_response(
    stream: &mut TcpStream,
    value: &next_rsc_flight::FlightValue,
    method: &str,
    runtime_data_accessed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let chunks = next_rsc_flight::encode_root_chunks_with_limits(
        value,
        next_rsc_flight::EncodeLimits::default(),
    )?;
    if method == "HEAD" {
        let content_length: usize = chunks.iter().map(Vec::len).sum();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/x-component\r\nContent-Length: \
             {content_length}\r\nCache-Control: {}\r\nVary: RSC, Next-Router-State-Tree, \
             Next-Router-Prefetch, Next-Router-Segment-Prefetch\r\nX-Content-Type-Options: \
             nosniff\r\nConnection: close\r\n\r\n",
            route_cache_control(runtime_data_accessed),
        )?;
        return Ok(());
    }

    stream.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/x-component\r\nTransfer-Encoding: \
             chunked\r\nCache-Control: {}\r\nVary: RSC, Next-Router-State-Tree, \
             Next-Router-Prefetch, Next-Router-Segment-Prefetch\r\nX-Content-Type-Options: \
             nosniff\r\nConnection: close\r\n\r\n",
            route_cache_control(runtime_data_accessed)
        )
        .as_bytes(),
    )?;
    for chunk in chunks {
        write!(stream, "{:x}\r\n", chunk.len())?;
        stream.write_all(&chunk)?;
        stream.write_all(b"\r\n")?;
    }
    stream.write_all(b"0\r\n\r\n")?;
    Ok(())
}

fn serve_static_asset(
    stream: &mut TcpStream,
    pathname: &str,
    method: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(relative) = safe_static_asset_relative(pathname) else {
        return write_response(
            stream,
            "400 Bad Request",
            "text/plain",
            b"Invalid asset path",
        );
    };
    let root = env::var("NEXT_STATIC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let production = PathBuf::from("rust-rsc-prototype/.next/static");
            if production.is_dir() {
                production
            } else {
                PathBuf::from("rust-rsc-prototype/.next/dev/static")
            }
        });
    let filename = root.join(relative);
    if !filename.starts_with(&root) || !filename.is_file() {
        return write_response(stream, "404 Not Found", "text/plain", b"Asset not found");
    }
    let body = fs::read(&filename)?;
    write_response_for_method(
        stream,
        "200 OK",
        asset_content_type(&filename),
        &body,
        method,
    )
}

fn safe_static_asset_relative(pathname: &str) -> Option<PathBuf> {
    let relative = pathname.strip_prefix("/_next/static/")?;
    let mut decoded = PathBuf::new();
    for segment in relative.split('/') {
        let segment = percent_decode_path_segment(segment)?;
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains(['/', '\\'])
        {
            return None;
        }
        decoded.push(segment);
    }
    Some(decoded)
}

fn asset_content_type(filename: &Path) -> &'static str {
    match filename.extension().and_then(|value| value.to_str()) {
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn public_asset_path(pathname: &str) -> Option<PathBuf> {
    let relative = pathname.strip_prefix('/')?;
    if relative.split('/').any(|segment| {
        segment.is_empty() || segment == "." || segment == ".." || segment.contains(['\\', '%'])
    }) {
        return None;
    }
    let root = env::var("NEXT_PUBLIC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("rust-rsc-prototype/public"));
    let filename = root.join(relative);
    (filename.starts_with(&root) && filename.is_file()).then_some(filename)
}

fn inject_app_router_bootstrap(
    html: &mut String,
    flight: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let flight = std::str::from_utf8(flight)?;
    let mut head = format!(
        "<script>self.__next_r=\"{RUST_RSC_BUILD_ID}\"</script><link rel=\"preload\" \
         as=\"script\" fetchpriority=\"low\" href=\"{RUST_RSC_WEBPACK_ASSET}\">"
    );
    push_native_metadata(&mut head);
    for asset in RUST_RSC_MAIN_ASSETS {
        write!(head, "<script src=\"{asset}\" async></script>").unwrap();
    }
    for asset in RUST_RSC_ENTRY_CLIENT_ASSETS {
        write!(head, "<script src=\"{asset}\" async></script>").unwrap();
    }
    for asset in RUST_RSC_POLYFILL_ASSETS {
        write!(head, "<script src=\"{asset}\" nomodule></script>").unwrap();
    }
    for asset in RUST_RSC_CSS_ASSETS {
        write!(head, "<link rel=\"stylesheet\" href=\"{asset}\">").unwrap();
    }
    if let Some(index) = html.find("</head>") {
        html.insert_str(index, &head);
    } else if let Some(index) = html.find("<body") {
        html.insert_str(index, &format!("<head>{head}</head>"));
    } else {
        return Err("native document is missing a body element".into());
    }

    let body_end = html
        .rfind("</body>")
        .ok_or("native document is missing a closing body element")?;
    let mut scripts = String::new();
    push_app_router_body_scripts(&mut scripts, flight);
    html.insert_str(body_end, &scripts);
    Ok(())
}

fn push_native_metadata(head: &mut String) {
    if !RUST_RSC_METADATA_TITLE.is_empty() {
        head.push_str("<title>");
        push_html_escaped(head, RUST_RSC_METADATA_TITLE);
        head.push_str("</title>");
    }
    if !RUST_RSC_METADATA_DESCRIPTION.is_empty() {
        head.push_str("<meta name=\"description\" content=\"");
        push_html_attribute_escaped(head, RUST_RSC_METADATA_DESCRIPTION);
        head.push_str("\">");
    }
    for (name, value) in [
        ("application-name", RUST_RSC_METADATA_APPLICATION_NAME),
        ("generator", RUST_RSC_METADATA_GENERATOR),
        ("referrer", RUST_RSC_METADATA_REFERRER),
        ("creator", RUST_RSC_METADATA_CREATOR),
        ("publisher", RUST_RSC_METADATA_PUBLISHER),
        ("category", RUST_RSC_METADATA_CATEGORY),
    ] {
        if !value.is_empty() {
            head.push_str("<meta name=\"");
            head.push_str(name);
            head.push_str("\" content=\"");
            push_html_attribute_escaped(head, value);
            head.push_str("\">");
        }
    }
}

fn push_app_router_body_scripts(output: &mut String, flight: &str) {
    output.push_str(&format!(
        "<script src=\"{RUST_RSC_WEBPACK_ASSET}\" \
         async></script><script>(self.__next_f=self.__next_f||[]).push([0])</script><script>self.\
         __next_f.push([1,"
    ));
    push_inline_json_string(output, flight);
    output.push_str("])</script>");
}

fn push_inline_json_string(output: &mut String, value: &str) {
    output.push('"');
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '<' => output.push_str("\\u003c"),
            '\u{2028}' => output.push_str("\\u2028"),
            '\u{2029}' => output.push_str("\\u2029"),
            ch if ch <= '\u{1f}' => write!(output, "\\u{:04x}", ch as u32).unwrap(),
            _ => output.push(ch),
        }
    }
    output.push('"');
}

fn read_request(stream: &mut TcpStream) -> Result<Vec<u8>, (&'static str, String)> {
    let mut request = Vec::with_capacity(4096);
    let mut buffer = [0_u8; 4096];
    let headers_end = loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|_| ("408 Request Timeout", "Request timed out".to_owned()))?;
        if count == 0 {
            return Err(("400 Bad Request", "Incomplete request".to_owned()));
        }
        request.extend_from_slice(&buffer[..count]);
        if let Some(index) = find_bytes(&request, b"\r\n\r\n") {
            break index + 4;
        }
        if request.len() > MAX_HEADER_BYTES {
            return Err((
                "431 Request Header Fields Too Large",
                "Request headers too large".to_owned(),
            ));
        }
    };
    if headers_end > MAX_HEADER_BYTES {
        return Err((
            "431 Request Header Fields Too Large",
            "Request headers too large".to_owned(),
        ));
    }
    let headers = std::str::from_utf8(&request[..headers_end]).map_err(|_| {
        (
            "400 Bad Request",
            "Request headers must be UTF-8".to_owned(),
        )
    })?;
    let mut content_length = None;
    for line in headers
        .split("\r\n")
        .skip(1)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(("400 Bad Request", "Obsolete folded header".to_owned()));
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(("400 Bad Request", "Malformed request header".to_owned()));
        };
        if !valid_http_token(name) {
            return Err(("400 Bad Request", "Invalid request header name".to_owned()));
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err((
                "400 Bad Request",
                "Transfer-Encoding is unsupported".to_owned(),
            ));
        }
        if name.eq_ignore_ascii_case("expect") {
            return Err(("417 Expectation Failed", "Expect is unsupported".to_owned()));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(("400 Bad Request", "Duplicate Content-Length".to_owned()));
            }
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(("400 Bad Request", "Invalid Content-Length".to_owned()));
            }
            content_length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| ("400 Bad Request", "Invalid Content-Length".to_owned()))?,
            );
        }
    }
    let content_length = content_length.unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(("413 Payload Too Large", "Request body too large".to_owned()));
    }
    let total_length = headers_end + content_length;
    while request.len() < total_length {
        let remaining = total_length - request.len();
        let read_length = remaining.min(buffer.len());
        let count = stream
            .read(&mut buffer[..read_length])
            .map_err(|_| ("408 Request Timeout", "Request timed out".to_owned()))?;
        if count == 0 {
            return Err(("400 Bad Request", "Incomplete request body".to_owned()));
        }
        request.extend_from_slice(&buffer[..count]);
    }
    request.truncate(total_length);
    Ok(request)
}

fn valid_http_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn write_render_error(
    stream: &mut TcpStream,
    error: RenderError,
    wants_flight: bool,
    method: &str,
    runtime_data_accessed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if wants_flight {
        return write_response_for_method_with_cache(
            stream,
            "200 OK",
            "text/x-component",
            &next_rsc_flight::encode_error(&error.flight_digest()),
            method,
            runtime_data_accessed,
        );
    }

    match error {
        RenderError::NotFound => write_response_for_method_with_cache(
            stream,
            "404 Not Found",
            "text/plain",
            b"Not found",
            method,
            runtime_data_accessed,
        ),
        RenderError::Redirect { location, status } => {
            if location.contains(['\r', '\n']) {
                return write_response(
                    stream,
                    "500 Internal Server Error",
                    "text/plain",
                    b"Invalid redirect location",
                );
            }
            write!(
                stream,
                "HTTP/1.1 {status} Temporary Redirect\r\nLocation: {location}\r\nContent-Length: \
                 0\r\nCache-Control: {}\r\nConnection: close\r\n\r\n",
                route_cache_control(runtime_data_accessed),
            )?;
            Ok(())
        }
        RenderError::Forbidden => write_response_for_method_with_cache(
            stream,
            "403 Forbidden",
            "text/plain",
            b"Forbidden",
            method,
            runtime_data_accessed,
        ),
        RenderError::Unauthorized => write_response_for_method_with_cache(
            stream,
            "401 Unauthorized",
            "text/plain",
            b"Unauthorized",
            method,
            runtime_data_accessed,
        ),
        RenderError::Message(message) => write_response_for_method_with_cache(
            stream,
            "500 Internal Server Error",
            "text/plain",
            message.as_bytes(),
            method,
            runtime_data_accessed,
        ),
        RenderError::HostFetch { .. } | RenderError::HostCacheTag { .. } => {
            write_response_for_method_with_cache(
                stream,
                "501 Not Implemented",
                "text/plain",
                b"Synchronous native components cannot request bridge host effects",
                method,
                runtime_data_accessed,
            )
        }
    }
}

fn proxy_to_fallback(
    client: &mut TcpStream,
    fallback_address: &str,
    request: &[u8],
    headers_end: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fallback = TcpStream::connect(fallback_address)?;
    fallback.set_read_timeout(Some(IO_TIMEOUT))?;
    fallback.set_write_timeout(Some(IO_TIMEOUT))?;
    let headers = std::str::from_utf8(&request[..headers_end])?;
    for (index, line) in headers.split("\r\n").enumerate() {
        if index == 0 {
            write!(fallback, "{line}\r\n")?;
            continue;
        }
        let Some((name, _)) = line.split_once(':') else {
            continue;
        };
        if is_internal_header(name) || name.eq_ignore_ascii_case("connection") {
            continue;
        }
        write!(fallback, "{line}\r\n")?;
    }
    fallback.write_all(b"Connection: close\r\n\r\n")?;
    fallback.write_all(&request[headers_end..])?;
    std::io::copy(&mut fallback, client)?;
    Ok(())
}

fn is_internal_header(name: &str) -> bool {
    const INTERNAL: &[&str] = &[
        "x-middleware-rewrite",
        "x-middleware-redirect",
        "x-middleware-set-cookie",
        "x-middleware-skip",
        "x-middleware-override-headers",
        "x-middleware-next",
        "x-now-route-matches",
        "x-matched-path",
        "x-nextjs-data",
        "x-next-resume-state-length",
        "next-resume",
    ];
    INTERNAL
        .iter()
        .any(|value| name.eq_ignore_ascii_case(value))
}

fn collect_request_data(headers: &[(&str, &str)]) -> next_rsc::RequestData {
    let mut request = next_rsc::RequestData::default();
    for (name, value) in headers {
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if is_internal_header(&name) {
            continue;
        }
        request
            .headers
            .entry(name.clone())
            .and_modify(|current| {
                current.push_str(", ");
                current.push_str(value);
            })
            .or_insert_with(|| value.to_owned());
        if name == "cookie" {
            for pair in value.split(';') {
                let Some((cookie_name, cookie_value)) = pair.trim().split_once('=') else {
                    continue;
                };
                if !cookie_name.is_empty() {
                    request
                        .cookies
                        .insert(cookie_name.to_owned(), cookie_value.to_owned());
                }
            }
        }
    }
    request
}

fn navigation_payload(
    pathname: &str,
    rendered_search: &str,
    tree_node: Node,
) -> next_rsc_flight::FlightValue {
    navigation_payload_with_seed(
        pathname,
        rendered_search,
        next_rsc_flight::FlightValue::Node(tree_node),
    )
}

fn navigation_payload_with_seed(
    pathname: &str,
    rendered_search: &str,
    seed_node: next_rsc_flight::FlightValue,
) -> next_rsc_flight::FlightValue {
    use next_rsc_flight::FlightValue::{
        Array, Bool, Null, Number, Object, String as FlightString, Undefined,
    };

    let route_segments: Vec<_> = pathname
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let route_pattern_segments: Vec<_> = native_ppr_route_pattern(pathname)
        .unwrap_or(pathname)
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let canonical_url_parts = Array(
        pathname
            .split('/')
            .map(|part| FlightString(part.to_owned()))
            .collect(),
    );
    let page_segment = if rendered_search.is_empty() {
        "__PAGE__".to_owned()
    } else {
        format!("__PAGE__?{}", search_params_json(rendered_search))
    };
    let mut children_tree = Array(vec![
        FlightString(page_segment),
        Object(Default::default()),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    for (index, segment) in route_segments.iter().enumerate().rev() {
        let mount = format!("/{}", route_segments[..=index].join("/"));
        let mut parallel_routes = BTreeMap::from([("children".to_owned(), children_tree)]);
        for (slot_mount, slot_name, slot_path) in native_parallel_slot_paths(pathname) {
            if *slot_mount == mount {
                parallel_routes.insert(
                    (*slot_name).to_owned(),
                    route_tree_for_segments(slot_path.split('/').filter(|part| !part.is_empty())),
                );
            }
        }
        children_tree = Array(vec![
            router_segment_value(
                route_pattern_segments
                    .get(index)
                    .copied()
                    .unwrap_or(segment),
                segment,
            ),
            Object(parallel_routes),
            Null,
            Null,
            Number(4096.0),
        ]);
    }
    let page_tree = Array(vec![
        FlightString("".to_owned()),
        Object(std::collections::BTreeMap::from([(
            "children".to_owned(),
            children_tree,
        )])),
        Null,
        Null,
        Number(4112.0),
    ]);

    let seed_data = Array(vec![
        seed_node,
        Object(Default::default()),
        Null,
        Bool(false),
        Null,
    ]);

    next_rsc_flight::FlightValue::object([
        ("c", canonical_url_parts),
        (
            "f",
            Array(vec![Array(vec![page_tree, seed_data, Null, Bool(false)])]),
        ),
        ("q", FlightString(rendered_search.to_owned())),
        ("i", Bool(false)),
        ("h", Null),
        ("r", Undefined),
        ("G", Array(vec![Null, Undefined])),
        ("S", Bool(false)),
        ("b", FlightString(RUST_RSC_BUILD_ID.to_owned())),
    ])
}

fn router_segment_value(pattern: &str, value: &str) -> next_rsc_flight::FlightValue {
    use next_rsc_flight::FlightValue::{Array, String as FlightString};
    let dynamic = if let Some(name) = pattern
        .strip_prefix("[[...")
        .and_then(|name| name.strip_suffix("]]"))
    {
        Some((name, "oc"))
    } else if let Some(name) = pattern
        .strip_prefix("[...")
        .and_then(|name| name.strip_suffix(']'))
    {
        Some((name, "c"))
    } else {
        pattern
            .strip_prefix('[')
            .and_then(|name| name.strip_suffix(']'))
            .map(|name| (name, "d"))
    };
    match dynamic {
        Some((name, kind)) => Array(vec![
            FlightString(name.to_owned()),
            FlightString(value.to_owned()),
            FlightString(kind.to_owned()),
            Array(Vec::new()),
        ]),
        None => FlightString(value.to_owned()),
    }
}

fn route_tree_for_segments<'a>(
    segments: impl DoubleEndedIterator<Item = &'a str>,
) -> next_rsc_flight::FlightValue {
    use next_rsc_flight::FlightValue::{
        Array, Null, Number, Object, String as FlightString, Undefined,
    };

    let mut tree = Array(vec![
        FlightString("__PAGE__".to_owned()),
        Object(Default::default()),
        Undefined,
        Undefined,
        Number(4096.0),
    ]);
    for segment in segments.rev() {
        tree = Array(vec![
            FlightString(segment.to_owned()),
            Object(BTreeMap::from([("children".to_owned(), tree)])),
            Null,
            Null,
            Number(4096.0),
        ]);
    }
    tree
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    write_response_for_method(stream, status, content_type, body, "GET")
}

fn route_cache_control(runtime_data_accessed: bool) -> &'static str {
    if runtime_data_accessed {
        "private, no-store"
    } else {
        "public, max-age=0, must-revalidate"
    }
}

fn write_response_for_method_with_cache(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    method: &str,
    runtime_data_accessed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: \
         {}\r\nCache-Control: {}\r\nVary: RSC, Next-Router-State-Tree, Next-Router-Prefetch, \
         Next-Router-Segment-Prefetch\r\nX-Content-Type-Options: nosniff\r\nConnection: \
         close\r\n\r\n",
        body.len(),
        route_cache_control(runtime_data_accessed),
    )?;
    if method != "HEAD" {
        stream.write_all(body)?;
    }
    Ok(())
}

fn write_response_for_method(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    method: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nVary: RSC, \
         Next-Router-State-Tree, Next-Router-Prefetch, \
         Next-Router-Segment-Prefetch\r\nX-Content-Type-Options: nosniff\r\nConnection: \
         close\r\n\r\n",
        body.len()
    )?;
    if method != "HEAD" {
        stream.write_all(body)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpListener, TcpStream},
        sync::mpsc,
        thread,
        time::Duration,
    };

    use crate::{
        ActiveRequest, MAX_ACTIVE_REQUESTS, NativeRewrite, RequestKind, apply_native_rewrite,
        classify_request, constant_time_equal, normalize_request_path, proxy_to_fallback,
        read_request, safe_static_asset_relative, trailing_slash_redirect,
    };

    fn parse_raw_request(raw: &'static [u8]) -> Result<Vec<u8>, (&'static str, String)> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(raw).unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
        });
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let result = read_request(&mut stream);
        writer.join().unwrap();
        result
    }

    #[test]
    fn request_parser_rejects_ambiguous_framing_and_malformed_headers() {
        for raw in [
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".as_slice(),
            b"POST / HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"GET / HTTP/1.1\r\n folded: value\r\n\r\n".as_slice(),
            b"GET / HTTP/1.1\r\nmalformed\r\n\r\n".as_slice(),
        ] {
            assert_eq!(parse_raw_request(raw).unwrap_err().0, "400 Bad Request");
        }
        assert_eq!(
            parse_raw_request(b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\nrust").unwrap(),
            b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\nrust"
        );
    }

    #[test]
    fn active_request_admission_is_bounded() {
        let active: Vec<_> = (0..MAX_ACTIVE_REQUESTS)
            .map(|_| ActiveRequest::try_acquire().unwrap())
            .collect();
        assert!(ActiveRequest::try_acquire().is_none());
        drop(active);
        assert!(ActiveRequest::try_acquire().is_some());
    }

    #[test]
    fn revalidation_token_comparison_checks_full_value() {
        assert!(constant_time_equal("local-secret", "local-secret"));
        assert!(!constant_time_equal("local-secreu", "local-secret"));
        assert!(!constant_time_equal("local-secret-extra", "local-secret"));
    }

    #[test]
    fn static_asset_paths_decode_route_segments_without_allowing_traversal() {
        assert_eq!(
            safe_static_asset_relative("/_next/static/chunks/app/catalog/product/%5Bid%5D/page.js"),
            Some("chunks/app/catalog/product/[id]/page.js".into())
        );
        assert!(safe_static_asset_relative("/_next/static/%2e%2e/secret").is_none());
        assert!(safe_static_asset_relative("/_next/static/chunks%2fsecret").is_none());
    }

    #[test]
    fn interception_state_falls_back_before_native_route_selection() {
        let interception = [
            ("RSC", "1"),
            (
                "Next-Router-State-Tree",
                r#"["",{"children":["gallery",{"modal":["(.)photo",{"children":["42",{}]}]}]}]"#,
            ),
        ];
        assert_eq!(
            classify_request(&interception),
            Ok(RequestKind::InterceptionNavigation)
        );

        let direct = [
            ("RSC", "1"),
            (
                "Next-Router-State-Tree",
                r#"["",{"children":["photo",{"children":["42",{}]}]}]"#,
            ),
        ];
        assert_eq!(classify_request(&direct), Ok(RequestKind::Navigation));
    }

    #[test]
    fn routing_configuration_normalizes_and_canonicalizes_paths() {
        assert_eq!(
            normalize_request_path("/dashboard/"),
            Some("/dashboard".to_owned())
        );
        assert_eq!(
            trailing_slash_redirect("/dashboard/?q=rust", "/dashboard/"),
            Some("/dashboard?q=rust".to_owned())
        );
        assert_eq!(trailing_slash_redirect("/asset.js", "/asset.js"), None);
        assert_eq!(
            apply_native_rewrite("/native-rust-page"),
            NativeRewrite::Internal("/rust-page".to_owned())
        );
        assert_eq!(
            apply_native_rewrite("/native-blog/fasteners"),
            NativeRewrite::Internal("/blog/fasteners".to_owned())
        );
        assert_eq!(
            apply_native_rewrite("/conditional-rust"),
            NativeRewrite::Fallback
        );
        assert_eq!(
            apply_native_rewrite("/external-docs/flight"),
            NativeRewrite::Fallback
        );
        assert_eq!(apply_native_rewrite("/rust-page"), NativeRewrite::None);
    }

    #[test]
    fn fallback_proxy_preserves_body_cookies_and_stream_bytes() {
        let fallback_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let fallback_address = fallback_listener.local_addr().unwrap();
        let fallback = thread::spawn(move || {
            let (mut stream, _) = fallback_listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..count]);
                if request.ends_with(b"payload") {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 201 Created\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\ndone\r\n0\r\n\r\n")
                .unwrap();
            request
        });

        let client_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(client_listener.local_addr().unwrap()).unwrap();
        let (mut client, _) = client_listener.accept().unwrap();
        let request = b"POST /fallback HTTP/1.1\r\nHost: example\r\nCookie: session=rust\r\nX-Matched-Path: secret\r\nContent-Length: 7\r\n\r\npayload";
        let headers_end = request
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap()
            + 4;
        proxy_to_fallback(
            &mut client,
            &fallback_address.to_string(),
            request,
            headers_end,
        )
        .unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = Vec::new();
        peer.read_to_end(&mut response).unwrap();
        let forwarded = fallback.join().unwrap();
        assert!(
            forwarded
                .windows(b"Cookie: session=rust".len())
                .any(|value| value == b"Cookie: session=rust")
        );
        assert!(
            !forwarded
                .windows(b"X-Matched-Path".len())
                .any(|value| value == b"X-Matched-Path")
        );
        assert!(forwarded.ends_with(b"\r\n\r\npayload"));
        assert_eq!(
            response
                .windows(b"Set-Cookie:".len())
                .filter(|value| *value == b"Set-Cookie:")
                .count(),
            2
        );
        assert!(response.ends_with(b"4\r\ndone\r\n0\r\n\r\n"));
    }

    #[test]
    fn fallback_proxy_closes_upstream_when_client_disconnects() {
        let fallback_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let fallback_address = fallback_listener.local_addr().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (closed_tx, closed_rx) = mpsc::channel();
        let fallback = thread::spawn(move || {
            let (mut stream, _) = fallback_listener.accept().unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .unwrap();
            started_tx.send(()).unwrap();
            let chunk = vec![b'x'; 64 * 1024];
            loop {
                if stream.write_all(&chunk).is_err() {
                    closed_tx.send(()).unwrap();
                    break;
                }
            }
        });

        let client_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(client_listener.local_addr().unwrap()).unwrap();
        let (mut client, _) = client_listener.accept().unwrap();
        let proxy = thread::spawn(move || {
            let request = b"GET /slow HTTP/1.1\r\nHost: example\r\n\r\n";
            let headers_end = request
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap()
                + 4;
            let _ = proxy_to_fallback(
                &mut client,
                &fallback_address.to_string(),
                request,
                headers_end,
            );
        });

        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        peer.shutdown(Shutdown::Both).unwrap();
        drop(peer);
        closed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        proxy.join().unwrap();
        fallback.join().unwrap();
    }
}

fn write_method_not_allowed(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    write!(
        stream,
        "HTTP/1.1 405 Method Not Allowed\r\nAllow: GET, HEAD\r\nContent-Type: \
         text/plain\r\nContent-Length: 18\r\nConnection: close\r\n\r\nMethod not allowed"
    )?;
    Ok(())
}

fn push_html(output: &mut String, node: &Node) -> Result<(), &'static str> {
    match node {
        Node::Null => {}
        Node::Text(value) => push_html_escaped(output, value),
        Node::Fragment(children) | Node::ReactFragment { children, .. } => {
            for child in children {
                push_html(output, child)?;
            }
        }
        Node::Element(element) => {
            if !valid_html_name(&element.tag) {
                return Err("native HTML prototype encountered invalid tag name");
            }
            output.push('<');
            output.push_str(&element.tag);
            for (name, value) in &element.props {
                let html_name = if name == "className" { "class" } else { name };
                if !valid_html_name(html_name) {
                    return Err("native HTML prototype encountered invalid attribute name");
                }
                match value {
                    PropValue::Bool(false) | PropValue::Null | PropValue::Undefined => continue,
                    PropValue::Bool(true) => {
                        output.push(' ');
                        output.push_str(html_name);
                    }
                    PropValue::String(value) => {
                        output.push(' ');
                        output.push_str(html_name);
                        output.push_str("=\"");
                        push_html_attribute_escaped(output, value);
                        output.push('"');
                    }
                    PropValue::Number(value) if value.is_finite() => {
                        write!(output, " {html_name}=\"{value}\"").unwrap();
                    }
                    PropValue::Strings(values) => {
                        output.push(' ');
                        output.push_str(html_name);
                        output.push_str("=\"");
                        push_html_attribute_escaped(output, &values.join(" "));
                        output.push('"');
                    }
                    PropValue::Style(styles) => {
                        output.push_str(" style=\"");
                        for (index, (property, value)) in styles.iter().enumerate() {
                            if index != 0 {
                                output.push(';');
                            }
                            push_css_property(output, property)?;
                            output.push(':');
                            push_html_attribute_escaped(output, value);
                        }
                        output.push('"');
                    }
                    PropValue::Number(_) => {
                        return Err("native HTML prototype encountered non-finite numeric prop");
                    }
                    PropValue::Node(_) => {
                        return Err("native HTML prototype encountered a node-valued attribute");
                    }
                }
            }
            output.push('>');
            if is_void_element(&element.tag) {
                if !element.children.is_empty() {
                    return Err("native HTML prototype encountered children on a void element");
                }
                return Ok(());
            }
            for child in &element.children {
                push_html(output, child)?;
            }
            output.push_str("</");
            output.push_str(&element.tag);
            output.push('>');
        }
        Node::Suspense { content, .. } => push_html(output, content)?,
        Node::Slot(_) => return Err("native HTML prototype encountered unresolved slot"),
        // Restricted native SSR supports transparent enhancer islands whose
        // server-visible output is exactly their intrinsic children.
        Node::ClientReference(reference) => {
            for child in &reference.children {
                push_html(output, child)?;
            }
        }
    }
    Ok(())
}

fn valid_html_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
}

fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn push_css_property(output: &mut String, property: &str) -> Result<(), &'static str> {
    if property.starts_with("--") {
        if !property
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("native HTML prototype encountered invalid CSS custom property");
        }
        output.push_str(property);
        return Ok(());
    }
    for (index, ch) in property.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                output.push('-');
            }
            output.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_alphanumeric() || ch == '-' {
            output.push(ch);
        } else {
            return Err("native HTML prototype encountered invalid style property");
        }
    }
    Ok(())
}

fn push_html_escaped(output: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(ch),
        }
    }
}

fn push_html_attribute_escaped(output: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '"' => output.push_str("&quot;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(ch),
        }
    }
}
