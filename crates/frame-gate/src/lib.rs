//! The only way out of a web compartment.
//!
//! A compartment has no network device. Its one path to the internet is a
//! channel to this gate, which speaks SOCKS5 because Chromium does. Everything
//! from the channel is hostile, including from our own guest code.
//!
//! The guarantee is structural: [`PublicAddr`] is the only thing that can open
//! a connection, and it cannot hold the host, the local network or any other
//! non-public address. There is no allow flag and no bypass to misconfigure.
//! Names are resolved here, and the address that was checked is the address
//! connected to, so a name cannot change its answer between the two.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{
    IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, SocketAddrV4, SocketAddrV6, TcpStream,
    ToSocketAddrs,
};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Longest host name in DNS.
pub const MAX_NAME: usize = 253;
/// How long a guest gets to finish its request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one connection attempt may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

const SOCKS_VERSION: u8 = 5;
const NO_AUTH: u8 = 0;
const NO_ACCEPTABLE_METHOD: u8 = 0xff;
const CMD_CONNECT: u8 = 1;
const ATYP_IPV4: u8 = 1;
const ATYP_NAME: u8 = 3;
const ATYP_IPV6: u8 = 4;
const REPLY_OK: u8 = 0;

/// Why the gate refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateError {
    /// The request broke the protocol. The sender is broken or hostile.
    Malformed,
    /// Anything but a TCP connect, such as bind or UDP.
    CommandRefused,
    /// The destination is not a public internet address.
    Forbidden,
    /// The destination is allowed but could not be reached.
    Unreachable,
}

impl GateError {
    fn reply_code(self) -> u8 {
        match self {
            Self::Malformed => 1,
            Self::Forbidden => 2,
            Self::Unreachable => 4,
            Self::CommandRefused => 7,
        }
    }
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gate refused: {self:?}")
    }
}

impl std::error::Error for GateError {}

fn ipv4_is_public(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !matches!(
        (a, b, c),
        (0 | 10 | 127 | 224..=255, _, _)
            | (100, 64..=127, _)
            | (169, 254, _)
            | (172, 16..=31, _)
            | (192, 0, 0 | 2)
            | (192, 88, 99)
            | (192, 168, _)
            | (198, 18..=19, _)
            | (198, 51, 100)
            | (203, 0, 113)
    )
}

/// Global unicast only, so mapped, NAT64 and local forms all fall outside it.
/// The tunnelling and documentation ranges inside it are refused as well.
fn ipv6_is_public(ip: Ipv6Addr) -> bool {
    let [a, b, ..] = ip.segments();
    let global_unicast = a & 0xe000 == 0x2000;
    let special = a == 0x2001 && (b < 0x0200 || b == 0x0db8);
    global_unicast && !special && a != 0x2002
}

/// A destination on the public internet, and the only way to open a
/// connection. It cannot hold the host, the local network or anything else
/// that is not public.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicAddr(SocketAddr);

impl PublicAddr {
    /// Returns `None` unless `address` is a public one with a real port.
    #[must_use]
    pub fn new(address: SocketAddr) -> Option<Self> {
        let public = match address.ip() {
            IpAddr::V4(ip) => ipv4_is_public(ip),
            IpAddr::V6(ip) => ipv6_is_public(ip),
        };
        (public && address.port() != 0).then_some(Self(address))
    }

    fn connect(self) -> io::Result<TcpStream> {
        TcpStream::connect_timeout(&self.0, CONNECT_TIMEOUT)
    }
}

/// What the guest asked to reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A literal address.
    Ip(SocketAddr),
    /// A host name and port. The name holds only DNS-safe ASCII.
    Name(String, u16),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ip(address) => write!(f, "{address}"),
            Self::Name(name, port) => write!(f, "{name}:{port}"),
        }
    }
}

/// Turns a host name into addresses.
pub trait Resolver {
    /// Resolves `host`.
    ///
    /// # Errors
    /// Any lookup failure.
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>>;
}

/// The host's resolver. The guest never learns which one that is.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        Ok((host, port).to_socket_addrs()?.collect())
    }
}

fn read_array<const N: usize>(stream: &mut impl Read) -> Result<[u8; N], GateError> {
    let mut bytes = [0; N];
    stream
        .read_exact(&mut bytes)
        .map_err(|_| GateError::Malformed)?;
    Ok(bytes)
}

fn name_is_valid(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn read_port(stream: &mut impl Read) -> Result<u16, GateError> {
    Ok(u16::from_be_bytes(read_array(stream)?))
}

fn read_name(stream: &mut impl Read) -> Result<Target, GateError> {
    let [length] = read_array(stream)?;
    let mut name = vec![0; usize::from(length)];
    stream
        .read_exact(&mut name)
        .map_err(|_| GateError::Malformed)?;
    if !name_is_valid(&name) {
        return Err(GateError::Malformed);
    }
    let port = read_port(stream)?;
    if port == 0 {
        return Err(GateError::Malformed);
    }
    let name = String::from_utf8(name).map_err(|_| GateError::Malformed)?;
    Ok(Target::Name(name, port))
}

/// Reads one SOCKS5 greeting and connect request.
///
/// # Errors
/// [`GateError::Malformed`] or [`GateError::CommandRefused`]. The channel
/// should be closed either way.
pub fn negotiate(stream: &mut (impl Read + Write)) -> Result<Target, GateError> {
    let [version, method_count] = read_array(stream)?;
    if version != SOCKS_VERSION || method_count == 0 {
        return Err(GateError::Malformed);
    }
    let mut methods = vec![0; usize::from(method_count)];
    stream
        .read_exact(&mut methods)
        .map_err(|_| GateError::Malformed)?;
    if !methods.contains(&NO_AUTH) {
        let _ = stream.write_all(&[SOCKS_VERSION, NO_ACCEPTABLE_METHOD]);
        return Err(GateError::Malformed);
    }
    stream
        .write_all(&[SOCKS_VERSION, NO_AUTH])
        .map_err(|_| GateError::Malformed)?;

    let [version, command, reserved, address_type] = read_array(stream)?;
    if version != SOCKS_VERSION || reserved != 0 {
        return Err(GateError::Malformed);
    }
    if command != CMD_CONNECT {
        return Err(GateError::CommandRefused);
    }
    match address_type {
        ATYP_IPV4 => {
            let ip = Ipv4Addr::from(read_array::<4>(stream)?);
            let port = read_port(stream)?;
            Ok(Target::Ip(SocketAddrV4::new(ip, port).into()))
        }
        ATYP_IPV6 => {
            let ip = Ipv6Addr::from(read_array::<16>(stream)?);
            let port = read_port(stream)?;
            Ok(Target::Ip(SocketAddrV6::new(ip, port, 0, 0).into()))
        }
        ATYP_NAME => read_name(stream),
        _ => Err(GateError::Malformed),
    }
}

/// Decides where a target may go. A name with even one non-public answer is
/// refused whole, since that is how DNS rebinding reaches a local network.
///
/// # Errors
/// [`GateError::Forbidden`] or [`GateError::Unreachable`].
pub fn route(target: &Target, resolver: &impl Resolver) -> Result<Vec<PublicAddr>, GateError> {
    let addresses = match target {
        Target::Ip(address) => vec![*address],
        Target::Name(name, port) => resolver
            .resolve(name, *port)
            .map_err(|_| GateError::Unreachable)?,
    };
    if addresses.is_empty() {
        return Err(GateError::Unreachable);
    }
    addresses
        .into_iter()
        .map(|address| PublicAddr::new(address).ok_or(GateError::Forbidden))
        .collect()
}

fn reply(stream: &mut impl Write, code: u8) -> Result<(), GateError> {
    stream
        .write_all(&[SOCKS_VERSION, code, 0, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
        .map_err(|_| GateError::Unreachable)
}

fn connect_first(routes: &[PublicAddr]) -> Result<TcpStream, GateError> {
    routes
        .iter()
        .find_map(|route| route.connect().ok())
        .ok_or(GateError::Unreachable)
}

fn relay(mut client: UnixStream, mut remote: TcpStream) -> Result<(), GateError> {
    let mut client_reader = client.try_clone().map_err(|_| GateError::Unreachable)?;
    let mut remote_writer = remote.try_clone().map_err(|_| GateError::Unreachable)?;
    let upload = std::thread::spawn(move || {
        let _ = io::copy(&mut client_reader, &mut remote_writer);
        let _ = remote_writer.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut remote, &mut client);
    let _ = client.shutdown(Shutdown::Both);
    let _ = upload.join();
    Ok(())
}

/// Serves one guest connection from request to close.
///
/// # Errors
/// The reason the request was refused. The guest has already been told.
pub fn handle(mut client: UnixStream, resolver: &impl Resolver) -> Result<(), GateError> {
    client
        .set_read_timeout(Some(REQUEST_TIMEOUT))
        .map_err(|_| GateError::Unreachable)?;
    let outcome = negotiate(&mut client).and_then(|target| {
        let connected = route(&target, resolver).and_then(|routes| connect_first(&routes));
        if matches!(connected, Err(GateError::Forbidden)) {
            eprintln!("GATE refused {target}");
        }
        connected
    });
    match outcome {
        Ok(remote) => {
            reply(&mut client, REPLY_OK)?;
            client
                .set_read_timeout(None)
                .map_err(|_| GateError::Unreachable)?;
            relay(client, remote)
        }
        Err(error) => {
            let _ = reply(&mut client, error.reply_code());
            Err(error)
        }
    }
}
