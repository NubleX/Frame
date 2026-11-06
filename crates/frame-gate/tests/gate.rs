//! The gate's promises, checked from the outside.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use frame_gate::{GateError, PublicAddr, Resolver, Target, handle, negotiate, route};
use std::io::{self, Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Read and write ends kept apart, so tests can inspect what the gate replied.
struct Pipe {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
}

impl Pipe {
    fn new(input: &[u8]) -> Self {
        Self {
            input: Cursor::new(input.to_vec()),
            output: Vec::new(),
        }
    }
}

impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.input.read(buffer)
    }
}

impl Write for Pipe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Fixed(Vec<SocketAddr>);

impl Resolver for Fixed {
    fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        Ok(self.0.clone())
    }
}

fn addr(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

fn request(tail: &[u8]) -> Vec<u8> {
    let mut bytes = vec![5, 1, 0, 5, 1, 0];
    bytes.extend_from_slice(tail);
    bytes
}

#[test]
fn every_non_public_address_is_unrepresentable() {
    for text in [
        "0.0.0.0:443",
        "0.1.2.3:443",
        "10.0.0.1:443",
        "100.64.0.5:443",
        "100.127.255.255:443",
        "127.0.0.1:443",
        "127.255.0.1:443",
        "169.254.169.254:80",
        "172.16.0.1:443",
        "172.31.255.255:443",
        "192.0.0.1:443",
        "192.0.2.1:443",
        "192.88.99.1:443",
        "192.168.1.185:443",
        "198.18.0.1:443",
        "198.19.255.255:443",
        "198.51.100.1:443",
        "203.0.113.1:443",
        "224.0.0.1:443",
        "239.255.255.250:1900",
        "240.0.0.1:443",
        "255.255.255.255:443",
        "[::]:443",
        "[::1]:443",
        "[::ffff:127.0.0.1]:443",
        "[::ffff:192.168.1.1]:443",
        "[::ffff:8.8.8.8]:443",
        "[64:ff9b::7f00:1]:443",
        "[fc00::1]:443",
        "[fd12:3456::1]:443",
        "[fe80::1]:443",
        "[ff02::1]:443",
        "[2001::1]:443",
        "[2001:db8::1]:443",
        "[2002:7f00:1::1]:443",
    ] {
        assert!(PublicAddr::new(addr(text)).is_none(), "{text} got through");
    }
}

#[test]
fn public_addresses_are_accepted() {
    for text in [
        "1.1.1.1:443",
        "9.9.9.9:53",
        "100.63.255.255:443",
        "100.128.0.1:443",
        "172.15.255.255:443",
        "172.32.0.1:443",
        "192.167.255.255:443",
        "198.17.255.255:443",
        "223.255.255.255:443",
        "[2606:4700:4700::1111]:443",
        "[2a01:4f8::1]:443",
    ] {
        assert!(PublicAddr::new(addr(text)).is_some(), "{text} refused");
    }
}

#[test]
fn port_zero_is_refused() {
    assert!(PublicAddr::new(addr("1.1.1.1:0")).is_none());
}

#[test]
fn a_name_resolving_to_any_private_address_is_refused() {
    let target = Target::Name("rebind.example".into(), 443);
    let resolver = Fixed(vec![addr("1.1.1.1:443"), addr("127.0.0.1:443")]);
    assert_eq!(route(&target, &resolver), Err(GateError::Forbidden));
}

#[test]
fn a_name_resolving_to_nothing_is_unreachable() {
    let target = Target::Name("none.example".into(), 443);
    assert_eq!(route(&target, &Fixed(vec![])), Err(GateError::Unreachable));
}

#[test]
fn a_public_name_routes_to_its_addresses() {
    let target = Target::Name("example.com".into(), 443);
    let routes = route(&target, &Fixed(vec![addr("1.1.1.1:443")])).unwrap();
    assert_eq!(routes.len(), 1);
}

#[test]
fn a_literal_private_address_is_refused_without_resolving() {
    let target = Target::Ip(addr("192.168.1.1:80"));
    assert_eq!(route(&target, &Fixed(vec![])), Err(GateError::Forbidden));
}

#[test]
fn accepts_connect_by_name() {
    let mut name = vec![3, 11];
    name.extend_from_slice(b"example.com");
    name.extend_from_slice(&443u16.to_be_bytes());
    let mut pipe = Pipe::new(&request(&name));
    assert_eq!(
        negotiate(&mut pipe),
        Ok(Target::Name("example.com".into(), 443))
    );
    assert_eq!(pipe.output, [5, 0]);
}

#[test]
fn accepts_connect_by_address() {
    let mut pipe = Pipe::new(&request(&[1, 1, 1, 1, 1, 1, 187]));
    assert_eq!(negotiate(&mut pipe), Ok(Target::Ip(addr("1.1.1.1:443"))));
}

#[test]
fn refuses_malformed_requests() {
    let long = [&[3u8, 254][..], &[b'a'; 254], &[1, 187]].concat();
    let cases: Vec<(Vec<u8>, GateError)> = vec![
        (vec![4, 1, 0], GateError::Malformed),
        (vec![5, 0], GateError::Malformed),
        (vec![5, 1, 2], GateError::Malformed),
        (vec![5, 1, 0, 4, 1, 0, 1], GateError::Malformed),
        (vec![5, 1, 0, 5, 1, 1, 1], GateError::Malformed),
        (vec![5, 1, 0, 5, 2, 0, 1], GateError::CommandRefused),
        (vec![5, 1, 0, 5, 3, 0, 1], GateError::CommandRefused),
        (vec![5, 1, 0, 5, 1, 0, 9], GateError::Malformed),
        (request(&[3, 0, 1, 187]), GateError::Malformed),
        (request(&long), GateError::Malformed),
        (
            request(&[3, 3, b'a', b'\n', b'b', 1, 187]),
            GateError::Malformed,
        ),
        (
            request(&[3, 3, 0xc3, 0xa9, b'b', 1, 187]),
            GateError::Malformed,
        ),
        (request(&[3, 1, b'a', 0, 0]), GateError::Malformed),
        (vec![5, 1], GateError::Malformed),
        (vec![], GateError::Malformed),
    ];
    for (bytes, expected) in cases {
        let mut pipe = Pipe::new(&bytes);
        assert_eq!(negotiate(&mut pipe), Err(expected), "{bytes:?}");
    }
}

/// End to end: a listener really exists on loopback, and the gate must not
/// connect to it however the guest asks.
#[test]
fn the_gate_never_reaches_a_real_loopback_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();

    let (mut guest, gate_side) = UnixStream::pair().unwrap();
    let worker = std::thread::spawn(move || handle(gate_side, &Fixed(vec![])));
    guest
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut literal = vec![5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1];
    literal.extend_from_slice(&port.to_be_bytes());
    guest.write_all(&literal).unwrap();

    let mut reply = [0; 12];
    guest.read_exact(&mut reply).unwrap();
    assert_eq!(&reply[..2], &[5, 0]);
    assert_eq!(reply[3], 2, "reply must be `not allowed`");
    assert_eq!(worker.join().unwrap(), Err(GateError::Forbidden));
    assert!(listener.accept().is_err(), "the gate touched loopback");
}
