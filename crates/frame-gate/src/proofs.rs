//! Machine-checked proofs, run with `cargo kani`. Unlike a test, each proof
//! covers every possible value of its inputs.
//!
//! The rules are written here a second way, as plain bit arithmetic, so a
//! mistake in the code cannot also hide in the proof.

use super::*;

const FORBIDDEN_V4: [(u32, u32); 14] = [
    (0x0000_0000, 8),  // this network
    (0x0a00_0000, 8),  // private
    (0x6440_0000, 10), // carrier-grade NAT
    (0x7f00_0000, 8),  // loopback
    (0xa9fe_0000, 16), // link-local, cloud metadata
    (0xac10_0000, 12), // private
    (0xc000_0000, 24), // protocol assignments
    (0xc000_0200, 24), // documentation
    (0xc058_6300, 24), // 6to4 relay
    (0xc0a8_0000, 16), // private
    (0xc612_0000, 15), // benchmarking
    (0xc633_6400, 24), // documentation
    (0xcb00_7100, 24), // documentation
    (0xe000_0000, 3),  // multicast and reserved
];

fn forbidden_v4(ip: u32) -> bool {
    FORBIDDEN_V4
        .iter()
        .any(|(network, bits)| ip >> (32 - bits) == network >> (32 - bits))
}

fn allowed_v6(ip: u128) -> bool {
    let global_unicast = ip >> 125 == 0b001;
    let ietf_special = ip >> (128 - 23) == 0x2001_0000 >> 9;
    let documentation = ip >> 96 == 0x2001_0db8;
    let six_to_four = ip >> 112 == 0x2002;
    global_unicast && !ietf_special && !documentation && !six_to_four
}

/// All 2^32 IPv4 addresses: public exactly when outside every forbidden range.
#[kani::proof]
fn every_ipv4_address_is_classified_correctly() {
    let ip: u32 = kani::any();
    assert_eq!(ipv4_is_public(Ipv4Addr::from(ip)), !forbidden_v4(ip));
}

/// All 2^128 IPv6 addresses.
#[kani::proof]
fn every_ipv6_address_is_classified_correctly() {
    let ip: u128 = kani::any();
    assert_eq!(ipv6_is_public(Ipv6Addr::from(ip)), allowed_v6(ip));
}

/// No value of the only type that can open a connection holds a forbidden
/// IPv4 address or port zero.
#[kani::proof]
fn public_addr_never_holds_a_forbidden_ipv4() {
    let ip: u32 = kani::any();
    let port: u16 = kani::any();
    let address = SocketAddrV4::new(Ipv4Addr::from(ip), port);
    if PublicAddr::new(address.into()).is_some() {
        assert!(!forbidden_v4(ip));
        assert!(port != 0);
    }
}

/// The same for IPv6.
#[kani::proof]
fn public_addr_never_holds_a_forbidden_ipv6() {
    let ip: u128 = kani::any();
    let port: u16 = kani::any();
    let address = SocketAddrV6::new(Ipv6Addr::from(ip), port, 0, 0);
    if PublicAddr::new(address.into()).is_some() {
        assert!(allowed_v6(ip));
        assert!(port != 0);
    }
}

/// Every accepted host name holds only safe bytes, so a log line cannot be
/// forged through one.
#[kani::proof]
#[kani::unwind(10)]
fn accepted_names_hold_only_safe_bytes() {
    let name: [u8; 8] = kani::any();
    let length: usize = kani::any();
    kani::assume(length <= name.len());
    let name = &name[..length];
    if name_is_valid(name) {
        assert!(!name.is_empty());
        for byte in name {
            assert!(*byte > b' ' && *byte < 0x7f);
        }
    }
}
