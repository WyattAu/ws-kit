#![no_main]

use libfuzzer_sys::fuzz_target;
use serde::{Deserialize, Serialize};
use ws_kit::codec::Codec;
use ws_kit::JsonCodec;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Msg {
    id: u64,
    body: String,
}

fuzz_target!(|data: &[u8]| {
    // Bound input so decode attempts stay fast (WebSocket frame cap).
    let s = String::from_utf8_lossy(&data[..data.len().min(64 * 1024)]);

    // Malformed or adversarial frames must return Err, never panic.
    let _ = <serde_json::Value as Codec>::decode(&s);
    let _ = Msg::decode(&s);
    let _ = JsonCodec::decode::<serde_json::Value>(&s);
    let _ = JsonCodec::decode::<Msg>(&s);

    // A successfully decoded value must re-encode without panicking.
    if let Ok(value) = <serde_json::Value as Codec>::decode(&s) {
        let re = value.encode_result();
        if let Ok(text) = re {
            let _ = <serde_json::Value as Codec>::decode(&text);
        }
    }

    // Deeply nested JSON must either decode or return Err — not overflow
    // in a way that panics the codec path.
    let _ = <serde_json::Value as Codec>::decode(&s);
});
