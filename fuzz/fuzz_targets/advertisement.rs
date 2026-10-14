#![no_main]
//! Fuzz the node-advertisement decoder: it must never panic on arbitrary bytes.
use libfuzzer_sys::fuzz_target;
use mesh_network::SignedNodeAdvertisement;

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<SignedNodeAdvertisement>(data);
});
