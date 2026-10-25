//! Hardware-independent logic for the 5G bridge: AT response parsing, SMS encoding,
//! call state, the iPhone JSON protocol and the call-audio wire format.
//!
//! Nothing here touches ESP-IDF, so it builds and tests on the host with `cargo test`.

pub mod at;
pub mod audio;
pub mod call;
pub mod framing;
pub mod power;
pub mod protocol;
pub mod sms;
pub mod ucs2;
