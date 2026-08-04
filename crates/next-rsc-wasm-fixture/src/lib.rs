//! Standalone ABI fixture used to exercise a Rust layout through WebAssembly.

use next_rsc::{LayoutProps, Node, Params, element, encode_render_result};

const MAX_INPUT_BYTES: usize = 64 * 1024;

#[unsafe(no_mangle)]
pub extern "C" fn next_rsc_alloc(len: usize) -> *mut u8 {
    let mut bytes = Vec::<u8>::with_capacity(len);
    let pointer = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    pointer
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn next_rsc_dealloc(pointer: *mut u8, len: usize) {
    if !pointer.is_null() && len != 0 {
        // SAFETY: the host only returns buffers allocated by next_rsc_alloc.
        drop(unsafe { Vec::from_raw_parts(pointer, 0, len) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn next_rsc_render(pointer: *mut u8, len: usize) -> u64 {
    let result = if pointer.is_null() || len > MAX_INPUT_BYTES {
        encode_render_result(Err(next_rsc::RenderError::new("invalid ABI input")))
    } else {
        // SAFETY: the host writes exactly len initialized bytes into this allocation.
        let input = unsafe { Vec::from_raw_parts(pointer, len, len) };
        render_input(&input)
    };
    return_bytes(result.into_bytes())
}

fn render_input(input: &[u8]) -> String {
    let Ok(input) = std::str::from_utf8(input) else {
        return encode_render_result(Err(next_rsc::RenderError::new("invalid UTF-8 ABI input")));
    };
    if !input.contains("\"abiVersion\":1") {
        return encode_render_result(Err(next_rsc::RenderError::new("ABI version mismatch")));
    }
    let Some(slug) = json_string_field(input, "slug") else {
        return encode_render_result(Err(next_rsc::RenderError::new("missing slug param")));
    };
    let props = LayoutProps::new(Node::slot(7), Params::default());
    encode_render_result(Ok(element(
        "section",
        [
            Node::text(format!("Wasm layout for {slug}")),
            props.children,
        ],
    )
    .prop("data-renderer", "wasm")))
}

fn json_string_field(input: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\":\"");
    let rest = input.split_once(&marker)?.1;
    let value = rest.split_once('"')?.0;
    (!value.contains(['\\', '\n', '\r'])).then(|| value.to_owned())
}

fn return_bytes(mut bytes: Vec<u8>) -> u64 {
    bytes.shrink_to_fit();
    let pointer = bytes.as_mut_ptr() as u32;
    let len = bytes.len() as u32;
    std::mem::forget(bytes);
    (u64::from(pointer) << 32) | u64::from(len)
}
