//! Property test: the advertisement decoder must never panic on arbitrary bytes.
use mesh_network::SignedNodeAdvertisement;
use proptest::prelude::*;

proptest! {
    #[test]
    fn advertisement_decoder_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = serde_json::from_slice::<SignedNodeAdvertisement>(&bytes);
    }
}
