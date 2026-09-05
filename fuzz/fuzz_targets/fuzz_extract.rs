#![no_main]

use libfuzzer_sys::fuzz_target;
use ws_kit::{TokenExtractor, TokenSourceKind};

fuzz_target!(|data: &[u8]| {
    // Bound input so extraction stays fast.
    let data = &data[..data.len().min(4096)];
    let s = String::from_utf8_lossy(data);

    // Split input into auth header / cookie header / query string thirds at
    // char boundaries.
    let mut a = s.len() / 3;
    let mut b = 2 * s.len() / 3;
    while a > 0 && !s.is_char_boundary(a) {
        a -= 1;
    }
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    let (auth, rest) = s.split_at(a);
    let (cookie, query) = rest.split_at(b - a);

    // Extraction is Option-valued and must never panic on adversarial input.
    let default = TokenExtractor::default();
    let _ = default.extract_from_parts(Some(auth), Some(cookie), query);
    let _ = default.extract_from_parts(None, None, query);
    let _ = default.extract_from_parts(Some(auth), None, &s);
    let _ = TokenExtractor::bearer_only().extract_from_parts(Some(auth), Some(cookie), query);

    // Every configured source kind, with names driven by the input.
    let name = query.trim().chars().take(16).collect::<String>();
    let all = TokenExtractor::new(vec![
        TokenSourceKind::AuthorizationHeader,
        TokenSourceKind::QueryParam(name.clone()),
        TokenSourceKind::Cookie(name),
    ]);
    let _ = all.extract_from_parts(Some(auth), Some(cookie), query);
    let _ = all.extract_from_parts(None, None, &s);
});
