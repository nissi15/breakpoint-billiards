//! Online play: two copies of the game meeting through a short room code.
//!
//! On the same network the guest finds the host by broadcasting the code and
//! connects directly over TCP. Otherwise both sides meet on a free public
//! MQTT relay, using topics derived from the code; no IP addresses, port
//! forwarding or accounts are needed.
//!
//! Only shots and decisions travel over the wire. Both sides run the same
//! deterministic physics, and after every shot the shooter also sends the
//! resting position of every ball, which the other side adopts, so the two
//! tables can never drift apart.

use crate::physics::ShotLog;
use crate::rules::ShotCall;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};


#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// Host to guest: game mode, rack seed and who breaks.
    Start {
        mode: u8,
        seed: u64,
        first: u8,
    },
    /// Live cue position while the remote player aims.
    Cue {
        x: f64,
        y: f64,
        aim: f64,
        power: f64,
        side: f64,
        top: f64,
    },
    Shot {
        x: f64,
        y: f64,
        angle: f64,
        speed: f64,
        side: f64,
        top: f64,
        call: ShotCall,
    },
    /// Resting state after a shot: id, on table?, x, y for every ball, plus
    /// the shooter's record of what happened, which both sides judge from.
    State(Vec<(u8, bool, f64, f64)>, ShotLog),
    Choice(u8),
    Bye,
}

fn f(v: f64) -> String {
    format!("{:x}", v.to_bits())
}

fn pf(s: Option<&str>) -> Option<f64> {
    u64::from_str_radix(s?, 16)
        .ok()
        .map(f64::from_bits)
        .filter(|v| v.is_finite())
}

impl Msg {
    pub fn encode(&self) -> String {
        match self {
            Msg::Start { mode, seed, first } => format!("START {mode} {seed:x} {first}"),
            Msg::Cue {
                x,
                y,
                aim,
                power,
                side,
                top,
            } => {
                format!(
                    "CUE {} {} {} {} {} {}",
                    f(*x),
                    f(*y),
                    f(*aim),
                    f(*power),
                    f(*side),
                    f(*top)
                )
            }
            Msg::Shot {
                x,
                y,
                angle,
                speed,
                side,
                top,
                call,
            } => format!(
                "SHOT {} {} {} {} {} {} {} {} {} {}",
                f(*x),
                f(*y),
                f(*angle),
                f(*speed),
                f(*side),
                f(*top),
                call.ball.map_or(-1, |b| b as i32),
                call.pocket.map_or(-1, |p| p as i32),
                call.safety as u8,
                call.push_out as u8
            ),
            Msg::State(balls, log) => {
                let mut s = format!(
                    "STATE {} {} {} {} {} {} {} {}",
                    log.first_hit.map_or(-1, |b| b as i32),
                    log.cue_potted as u8,
                    log.cushion_after_contact as u8,
                    log.balls_to_cushion,
                    log.crossed_head_string as u8,
                    log.first_hit_in_kitchen as u8,
                    log.potted.len(),
                    balls.len()
                );
                for (b, p) in log.potted.iter().zip(log.pockets.iter()) {
                    s.push_str(&format!(" {b} {p}"));
                }
                for (id, on, x, y) in balls {
                    s.push_str(&format!(" {id} {} {} {}", *on as u8, f(*x), f(*y)));
                }
                s
            }
            Msg::Choice(c) => format!("CHOICE {c}"),
            Msg::Bye => "BYE".into(),
        }
    }

    pub fn decode(line: &str) -> Option<Msg> {
        let mut it = line.split_whitespace();
        match it.next()? {
            "START" => Some(Msg::Start {
                mode: it.next()?.parse().ok()?,
                seed: u64::from_str_radix(it.next()?, 16).ok()?,
                first: it.next()?.parse().ok()?,
            }),
            "CUE" => Some(Msg::Cue {
                x: pf(it.next())?,
                y: pf(it.next())?,
                aim: pf(it.next())?,
                power: pf(it.next())?,
                side: pf(it.next())?,
                top: pf(it.next())?,
            }),
            "SHOT" => {
                let (x, y, angle, speed, side, top) = (
                    pf(it.next())?,
                    pf(it.next())?,
                    pf(it.next())?,
                    pf(it.next())?,
                    pf(it.next())?,
                    pf(it.next())?,
                );
                let ball: i32 = it.next()?.parse().ok()?;
                let pocket: i32 = it.next()?.parse().ok()?;
                let safety = it.next()? == "1";
                let push_out = it.next()? == "1";
                Some(Msg::Shot {
                    x,
                    y,
                    angle,
                    speed,
                    side,
                    top,
                    call: ShotCall {
                        ball: (0..=15).contains(&ball).then_some(ball as u8),
                        pocket: (0..6).contains(&pocket).then_some(pocket as usize),
                        safety,
                        push_out,
                    },
                })
            }
            "STATE" => {
                let first: i32 = it.next()?.parse().ok()?;
                let mut log = ShotLog {
                    first_hit: (1..=15).contains(&first).then_some(first as u8),
                    cue_potted: it.next()? == "1",
                    cushion_after_contact: it.next()? == "1",
                    balls_to_cushion: it.next()?.parse().ok()?,
                    crossed_head_string: it.next()? == "1",
                    first_hit_in_kitchen: it.next()? == "1",
                    ..ShotLog::default()
                };
                let potted: usize = it.next()?.parse().ok()?;
                let count: usize = it.next()?.parse().ok()?;
                if potted > 15 || count > 16 {
                    return None;
                }
                for _ in 0..potted {
                    let b: u8 = it.next()?.parse().ok()?;
                    let p: usize = it.next()?.parse().ok()?;
                    if !(1..=15).contains(&b) || p >= 6 {
                        return None;
                    }
                    log.potted.push(b);
                    log.pockets.push(p);
                }
                let mut balls = Vec::new();
                for _ in 0..count {
                    let id: u8 = it.next()?.parse().ok()?;
                    if id > 15 {
                        return None;
                    }
                    balls.push((id, it.next()? == "1", pf(it.next())?, pf(it.next())?));
                }
                Some(Msg::State(balls, log))
            }
            "CHOICE" => Some(Msg::Choice(it.next()?.parse().ok()?)),
            "BYE" => Some(Msg::Bye),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Room codes
// ---------------------------------------------------------------------------

/// UDP port used to find a room on the local network.
pub const LAN_PORT: u16 = 47801;
/// Characters used in room codes: no I, O, 0 or 1, so codes read out loud
/// and typed from a screenshot never get confused.
pub const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 6;
/// Free public relays. The room code itself decides which one a room uses,
/// so both players always meet on the same relay.
const RELAYS: [(&str, u16); 2] = [("broker.emqx.io", 1883), ("test.mosquitto.org", 1883)];
const TOPIC_ROOT: &str = "breakpoint-billiards/v1";
const PING_EVERY: Duration = Duration::from_secs(2);
/// A peer that has said nothing for this long is gone.
const SILENCE_LIMIT: Duration = Duration::from_secs(15);

fn fnv(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

fn relay_for(code: &str) -> usize {
    (fnv(code) % RELAYS.len() as u64) as usize
}

fn random_u64() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(7);
    // Mix in the address of a stack value so two instances started in the
    // same nanosecond still differ.
    let local = 0u8;
    let addr = &local as *const u8 as u64;
    let mut x = t ^ addr.rotate_left(32) ^ 0x9E3779B97F4A7C15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^ (x >> 33)
}

fn random_code(relay: usize) -> String {
    let mut seed = random_u64();
    loop {
        let mut code = String::new();
        for _ in 0..CODE_LEN {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            code.push(CODE_ALPHABET[(seed >> 59) as usize % CODE_ALPHABET.len()] as char);
        }
        if relay_for(&code) == relay {
            return code;
        }
    }
}

fn random_id() -> String {
    format!("{:012x}", random_u64() & 0xffff_ffff_ffff)
}

/// Clean up what a player typed or pasted: upper case, separators removed.
/// Returns None unless it is a complete, valid code.
pub fn normalize_code(input: &str) -> Option<String> {
    let code: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let valid = code.len() == CODE_LEN && code.bytes().all(|b| CODE_ALPHABET.contains(&b));
    valid.then_some(code)
}

fn topic(code: &str, to: &str) -> String {
    format!("{TOPIC_ROOT}/{code}/{to}")
}

// ---------------------------------------------------------------------------
// A tiny MQTT 3.1.1 client: just enough to talk through a public relay.
// ---------------------------------------------------------------------------

fn put_str(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s);
}

fn packet(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![kind];
    let mut len = body.len();
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            break;
        }
    }
    out.extend_from_slice(body);
    out
}

/// Split one complete packet off the front of `buf`: (first byte, body).
fn take_packet(buf: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    let mut len = 0usize;
    let mut mult = 1usize;
    let mut i = 1;
    loop {
        let b = *buf.get(i)?;
        len += (b & 0x7f) as usize * mult;
        i += 1;
        if b & 0x80 == 0 {
            break;
        }
        mult *= 128;
        if i > 4 {
            // Malformed length: drop everything rather than spin.
            buf.clear();
            return None;
        }
    }
    if buf.len() < i + len {
        return None;
    }
    let first = buf[0];
    let body = buf[i..i + len].to_vec();
    buf.drain(..i + len);
    Some((first, body))
}

fn write_fully(stream: &mut TcpStream, data: &[u8]) -> bool {
    let mut sent = 0;
    let mut waits = 0;
    while sent < data.len() {
        match stream.write(&data[sent..]) {
            Ok(0) => return false,
            Ok(n) => sent += n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                waits += 1;
                if waits > 2000 {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => return false,
        }
    }
    true
}

struct Mqtt {
    stream: TcpStream,
    buf: Vec<u8>,
    /// Messages read during a handshake but meant for whoever uses the
    /// connection next.
    backlog: Vec<String>,
    last_tx: Instant,
    alive: bool,
}

impl Mqtt {
    /// Connect, set a last-will message, subscribe and switch to
    /// non-blocking mode. Blocks for at most a few seconds.
    fn connect(relay: usize, will: (&str, &str), subscribe: &str) -> Result<Mqtt, String> {
        let (host, port) = RELAYS[relay];
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|_| "No internet connection".to_string())?
            .next()
            .ok_or("No internet connection")?;
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(4))
            .map_err(|_| "Could not reach the online relay".to_string())?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut body = Vec::new();
        put_str(&mut body, b"MQTT");
        body.push(4); // protocol level 3.1.1
        body.push(0x02 | 0x04); // clean session, will flag
        body.extend_from_slice(&60u16.to_be_bytes()); // keep-alive seconds
        put_str(&mut body, format!("bpb-{}", random_id()).as_bytes());
        put_str(&mut body, will.0.as_bytes());
        put_str(&mut body, will.1.as_bytes());
        stream.write_all(&packet(0x10, &body)).map_err(|e| e.to_string())?;
        let mut ack = [0u8; 4];
        stream
            .read_exact(&mut ack)
            .map_err(|_| "The online relay did not answer".to_string())?;
        if ack[0] != 0x20 || ack[3] != 0 {
            return Err("The online relay refused the connection".into());
        }
        let mut body = 1u16.to_be_bytes().to_vec();
        put_str(&mut body, subscribe.as_bytes());
        body.push(0); // QoS 0
        stream.write_all(&packet(0x82, &body)).map_err(|e| e.to_string())?;
        let mut m = Mqtt {
            stream,
            buf: Vec::new(),
            backlog: Vec::new(),
            last_tx: Instant::now(),
            alive: true,
        };
        // Wait for SUBACK so nothing published to us afterwards is missed.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((first, _)) = take_packet(&mut m.buf) {
                if first >> 4 == 9 {
                    break;
                }
                continue;
            }
            if Instant::now() > deadline {
                return Err("The online relay did not answer".into());
            }
            let mut chunk = [0u8; 512];
            match m.stream.read(&mut chunk) {
                Ok(0) | Err(_) => return Err("The online relay closed the connection".into()),
                Ok(n) => m.buf.extend_from_slice(&chunk[..n]),
            }
        }
        m.stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(m)
    }

    fn publish(&mut self, topic: &str, payload: &str) {
        if !self.alive {
            return;
        }
        let mut body = Vec::new();
        put_str(&mut body, topic.as_bytes());
        body.extend_from_slice(payload.as_bytes());
        self.alive = write_fully(&mut self.stream, &packet(0x30, &body));
        self.last_tx = Instant::now();
    }

    /// Payloads of every message received since the last call.
    fn poll(&mut self) -> Vec<String> {
        let mut out = std::mem::take(&mut self.backlog);
        if !self.alive {
            return out;
        }
        if self.last_tx.elapsed() > Duration::from_secs(20) {
            self.alive = write_fully(&mut self.stream, &[0xC0, 0x00]);
            self.last_tx = Instant::now();
        }
        let mut chunk = [0u8; 8192];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.alive = false;
                    break;
                }
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.alive = false;
                    break;
                }
            }
        }
        while let Some((first, body)) = take_packet(&mut self.buf) {
            if first >> 4 != 3 || body.len() < 2 {
                continue;
            }
            let tlen = u16::from_be_bytes([body[0], body[1]]) as usize;
            let mut start = 2 + tlen;
            if (first >> 1) & 3 > 0 {
                start += 2; // packet id, only present above QoS 0
            }
            if let Some(p) = body.get(start..) {
                if let Ok(s) = std::str::from_utf8(p) {
                    out.push(s.to_string());
                }
            }
        }
        out
    }

    fn disconnect(&mut self) {
        if self.alive {
            let _ = write_fully(&mut self.stream, &[0xE0, 0x00]);
            self.alive = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Peer: the other player, reached directly or through the relay.
// ---------------------------------------------------------------------------

enum Link {
    Direct { stream: TcpStream, buf: Vec<u8> },
    Relay { mqtt: Mqtt, out: String, me: String, them: String },
}

pub struct Peer {
    link: Link,
    pub alive: bool,
    last_heard: Instant,
    last_ping: Instant,
    /// Round-trip time of the last ping, in milliseconds.
    pub rtt_ms: Option<u32>,
    epoch: Instant,
    // Guaranteed delivery. The relay may drop a message now and then, so
    // every message that matters is numbered, acknowledged by the other side
    // and re-sent until it is. Live aiming updates skip this: a newer one is
    // always on its way.
    next_seq: u64,
    unacked: std::collections::VecDeque<(u64, String, Instant)>,
    /// Next sequence number expected from the other side.
    expected: u64,
    /// Messages that arrived ahead of a missing one.
    held: std::collections::BTreeMap<u64, String>,
    /// Testing aid: pretend the next N numbered messages were lost.
    #[cfg(test)]
    drop_next: u32,
}

impl Peer {
    fn direct(stream: TcpStream) -> Peer {
        let _ = stream.set_nodelay(true);
        let _ = stream.set_nonblocking(true);
        Peer::with(Link::Direct { stream, buf: Vec::new() })
    }

    fn with(link: Link) -> Peer {
        Peer {
            link,
            alive: true,
            last_heard: Instant::now(),
            last_ping: Instant::now() - PING_EVERY,
            rtt_ms: None,
            epoch: Instant::now(),
            next_seq: 1,
            unacked: std::collections::VecDeque::new(),
            expected: 1,
            held: std::collections::BTreeMap::new(),
            #[cfg(test)]
            drop_next: 0,
        }
    }

    fn retry_after(&self) -> Duration {
        let rtt = self.rtt_ms.unwrap_or(300).clamp(50, 3000) as u64;
        Duration::from_millis(250 + rtt * 2)
    }

    fn transmit(&mut self, seq: u64, line: &str) {
        #[cfg(test)]
        if self.drop_next > 0 {
            self.drop_next -= 1;
            return;
        }
        self.send_line(&format!("R {seq} {line}"));
    }

    /// True when messages go through the internet relay rather than the LAN.
    pub fn relayed(&self) -> bool {
        matches!(self.link, Link::Relay { .. })
    }

    fn send_line(&mut self, line: &str) {
        if !self.alive {
            return;
        }
        match &mut self.link {
            Link::Direct { stream, .. } => {
                let mut data = line.as_bytes().to_vec();
                data.push(b'\n');
                self.alive = write_fully(stream, &data);
            }
            Link::Relay { mqtt, out, me, .. } => {
                let payload = format!("{me} {line}");
                let topic = out.clone();
                mqtt.publish(&topic, &payload);
                self.alive = mqtt.alive;
            }
        }
    }

    pub fn send(&mut self, msg: &Msg) {
        let line = msg.encode();
        if matches!(msg, Msg::Cue { .. }) {
            self.send_line(&line);
            return;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.unacked.push_back((seq, line.clone(), Instant::now()));
        self.transmit(seq, &line);
    }

    /// Re-send anything the other side has not confirmed yet.
    fn resend_overdue(&mut self) {
        let wait = self.retry_after();
        let due: Vec<(u64, String)> = self
            .unacked
            .iter()
            .filter(|(_, _, at)| at.elapsed() >= wait)
            .map(|(seq, line, _)| (*seq, line.clone()))
            .collect();
        for (seq, line) in due {
            self.transmit(seq, &line);
            if let Some(entry) = self.unacked.iter_mut().find(|e| e.0 == seq) {
                entry.2 = Instant::now();
            }
        }
    }

    fn deliver(line: &str, out: &mut Vec<Msg>, alive: &mut bool) {
        if let Some(m) = Msg::decode(line) {
            if m == Msg::Bye {
                *alive = false;
            }
            out.push(m);
        }
    }

    fn incoming_lines(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        match &mut self.link {
            Link::Direct { stream, buf } => {
                let mut chunk = [0u8; 8192];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => {
                            self.alive = false;
                            break;
                        }
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => {
                            self.alive = false;
                            break;
                        }
                    }
                    if buf.len() > 1 << 20 {
                        self.alive = false;
                        break;
                    }
                }
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    if let Ok(s) = std::str::from_utf8(&line) {
                        lines.push(s.trim_end().to_string());
                    }
                }
            }
            Link::Relay { mqtt, them, .. } => {
                for payload in mqtt.poll() {
                    // Every relayed message starts with its sender's id;
                    // anything from a stranger who guessed the code is ignored.
                    if let Some((from, rest)) = payload.split_once(' ') {
                        if from == them {
                            lines.push(rest.to_string());
                        }
                    }
                }
                if !mqtt.alive {
                    self.alive = false;
                }
            }
        }
        lines
    }

    /// Everything that has arrived since the last call. Also keeps the
    /// connection alive and notices when the other player disappears.
    pub fn poll(&mut self) -> Vec<Msg> {
        let mut out = Vec::new();
        if !self.alive {
            return out;
        }
        if self.last_ping.elapsed() >= PING_EVERY {
            self.last_ping = Instant::now();
            let now = self.epoch.elapsed().as_millis() as u64;
            self.send_line(&format!("PING {now}"));
        }
        for line in self.incoming_lines() {
            self.last_heard = Instant::now();
            let mut words = line.split_whitespace();
            match words.next() {
                Some("PING") => {
                    let stamp = words.next().unwrap_or("0").to_string();
                    self.send_line(&format!("PONG {stamp}"));
                }
                Some("PONG") => {
                    if let Some(sent) = words.next().and_then(|w| w.parse::<u64>().ok()) {
                        let now = self.epoch.elapsed().as_millis() as u64;
                        self.rtt_ms = Some(now.saturating_sub(sent) as u32);
                    }
                }
                Some("R") => {
                    let Some(seq) = words.next().and_then(|w| w.parse::<u64>().ok()) else { continue };
                    let body = line.splitn(3, ' ').nth(2).unwrap_or("").to_string();
                    if seq == self.expected {
                        Self::deliver(&body, &mut out, &mut self.alive);
                        self.expected += 1;
                        while let Some(next) = self.held.remove(&self.expected) {
                            Self::deliver(&next, &mut out, &mut self.alive);
                            self.expected += 1;
                        }
                    } else if seq > self.expected && self.held.len() < 512 {
                        self.held.insert(seq, body);
                    }
                    // Confirm everything received in order so far.
                    let upto = self.expected - 1;
                    self.send_line(&format!("A {upto}"));
                }
                Some("A") => {
                    if let Some(upto) = words.next().and_then(|w| w.parse::<u64>().ok()) {
                        while self.unacked.front().is_some_and(|e| e.0 <= upto) {
                            self.unacked.pop_front();
                        }
                    }
                }
                _ => Self::deliver(&line, &mut out, &mut self.alive),
            }
        }
        if !self.unacked.is_empty() {
            self.resend_overdue();
        }
        if self.last_heard.elapsed() > SILENCE_LIMIT {
            self.alive = false;
        }
        out
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        if self.alive {
            self.send_line("BYE");
        }
        if let Link::Relay { mqtt, .. } = &mut self.link {
            // Close without MQTT's polite DISCONNECT: the relay then also
            // publishes our last-will BYE, so the other player hears we left
            // even if the explicit BYE above is still in flight.
            let _ = mqtt.stream.shutdown(std::net::Shutdown::Both);
            mqtt.alive = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Rooms
// ---------------------------------------------------------------------------

/// Which ways of meeting are allowed. Both are on in the game; tests switch
/// one off to check the other on its own.
#[derive(Clone, Copy, Debug)]
pub struct Paths {
    pub lan: bool,
    pub relay: bool,
}

impl Paths {
    pub fn all() -> Paths {
        let relay_only = std::env::var("BPB_RELAY_ONLY").is_ok();
        let lan_only = std::env::var("BPB_LAN_ONLY").is_ok();
        Paths { lan: !relay_only, relay: !lan_only }
    }
}

/// A room the host has opened and is waiting in.
pub struct Room {
    pub code: String,
    /// Friends on other networks can join through the relay.
    pub online: bool,
    /// Friends on this network can join directly.
    pub lan: bool,
    listener: Option<TcpListener>,
    udp: Option<UdpSocket>,
    relay: Option<Mqtt>,
    id: String,
}

/// Open a room in the background; the result arrives on the channel.
pub fn create_room(paths: Paths) -> Receiver<Result<Room, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(open_room(paths));
    });
    rx
}

/// Relay indices ordered by how quickly they accept a connection from here.
fn relays_by_speed() -> Vec<usize> {
    let (tx, rx) = channel();
    for (i, (host, port)) in RELAYS.iter().enumerate() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let ok = (*host, *port)
                .to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next())
                .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).ok())
                .is_some();
            let _ = tx.send((i, ok, start.elapsed()));
        });
    }
    drop(tx);
    let mut timed: Vec<(usize, bool, Duration)> = rx.iter().collect();
    timed.sort_by_key(|(i, ok, t)| (!ok, *t, *i));
    let mut order: Vec<usize> = timed.into_iter().map(|(i, _, _)| i).collect();
    for i in 0..RELAYS.len() {
        if !order.contains(&i) {
            order.push(i);
        }
    }
    order
}

fn open_room(paths: Paths) -> Result<Room, String> {
    let id = random_id();
    // Use the first relay that answers; the code is chosen to point at it.
    let mut relay = None;
    let mut code = random_code(0);
    if paths.relay {
        for index in relays_by_speed() {
            let candidate = random_code(index);
            let will = (topic(&candidate, "g"), format!("{id} BYE"));
            if let Ok(m) = Mqtt::connect(index, (&will.0, &will.1), &topic(&candidate, "h")) {
                relay = Some(m);
                code = candidate;
                break;
            }
        }
    }
    let mut listener = None;
    let mut udp = None;
    if paths.lan {
        if let Ok(l) = TcpListener::bind(("0.0.0.0", 0)) {
            let _ = l.set_nonblocking(true);
            listener = Some(l);
        }
        if let Ok(u) = UdpSocket::bind(("0.0.0.0", LAN_PORT)) {
            let _ = u.set_nonblocking(true);
            udp = Some(u);
        }
    }
    if relay.is_none() && listener.is_none() {
        return Err("Could not open a room - check your internet connection".into());
    }
    Ok(Room {
        code,
        online: relay.is_some(),
        lan: listener.is_some() && udp.is_some(),
        listener,
        udp,
        relay,
        id,
    })
}

impl Room {
    /// Answer anyone looking for the room; returns the guest once one arrives.
    pub fn poll(&mut self) -> Option<Peer> {
        if let (Some(udp), Some(listener)) = (&self.udp, &self.listener) {
            let mut buf = [0u8; 256];
            while let Ok((n, from)) = udp.recv_from(&mut buf) {
                let msg = String::from_utf8_lossy(&buf[..n]).to_string();
                if msg.trim() == format!("BPB1 FIND {}", self.code) {
                    if let Ok(addr) = listener.local_addr() {
                        let reply = format!("BPB1 HERE {} {}", self.code, addr.port());
                        let _ = udp.send_to(reply.as_bytes(), from);
                    }
                }
            }
        }
        if let Some(listener) = &self.listener {
            if let Ok((mut stream, _)) = listener.accept() {
                // The guest proves it knows the code before play starts.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
                let mut line = [0u8; 64];
                let mut got = Vec::new();
                while !got.contains(&b'\n') && got.len() < 64 {
                    match stream.read(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got.extend_from_slice(&line[..n]),
                    }
                }
                if String::from_utf8_lossy(&got).trim() == format!("BPB1 JOIN {}", self.code)
                    && stream.write_all(b"BPB1 OK\n").is_ok()
                {
                    if let Some(m) = self.relay.as_mut() {
                        m.disconnect();
                    }
                    return Some(Peer::direct(stream));
                }
            }
        }
        if let Some(m) = self.relay.as_mut() {
            let payloads = m.poll();
            for (i, payload) in payloads.iter().enumerate() {
                if let Some((guest, "HELLO")) = payload.split_once(' ') {
                    let guest = guest.to_string();
                    let welcome = format!("{} WELCOME {guest}", self.id);
                    m.publish(&topic(&self.code, "g"), &welcome);
                    m.backlog = payloads[i + 1..].to_vec();
                    let mqtt = self.relay.take().unwrap();
                    return Some(Peer::with(Link::Relay {
                        mqtt,
                        out: topic(&self.code, "g"),
                        me: self.id.clone(),
                        them: guest,
                    }));
                }
            }
            if !m.alive {
                self.relay = None;
                self.online = false;
            }
        }
        None
    }
}

impl Drop for Room {
    fn drop(&mut self) {
        if let Some(m) = self.relay.as_mut() {
            m.disconnect();
        }
    }
}

/// Join a room by its code in the background.
pub fn join_room(code: &str, paths: Paths) -> Receiver<Result<Peer, String>> {
    let (tx, rx) = channel();
    let code = normalize_code(code);
    std::thread::spawn(move || {
        let result = match code {
            None => Err("Room codes are 6 letters and numbers".to_string()),
            Some(code) => find_room(&code, paths),
        };
        let _ = tx.send(result);
    });
    rx
}

fn find_on_lan(code: &str) -> Option<Peer> {
    let udp = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    let _ = udp.set_broadcast(true);
    let _ = udp.set_read_timeout(Some(Duration::from_millis(250)));
    let ask = format!("BPB1 FIND {code}");
    for _ in 0..3 {
        for target in [("255.255.255.255", LAN_PORT), ("127.0.0.1", LAN_PORT)] {
            let _ = udp.send_to(ask.as_bytes(), target);
        }
        let mut buf = [0u8; 256];
        while let Ok((n, from)) = udp.recv_from(&mut buf) {
            let reply = String::from_utf8_lossy(&buf[..n]).to_string();
            let mut words = reply.split_whitespace();
            let ok = words.next() == Some("BPB1") && words.next() == Some("HERE") && words.next() == Some(code);
            let Some(port) = words.next().and_then(|p| p.parse::<u16>().ok()).filter(|_| ok) else { continue };
            let addr = std::net::SocketAddr::new(from.ip(), port);
            let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            if stream.write_all(format!("BPB1 JOIN {code}\n").as_bytes()).is_err() {
                continue;
            }
            // Read the OK line one byte at a time, so the host's first game
            // message stays in the socket for the game to read.
            let mut got = Vec::new();
            let mut byte = [0u8; 1];
            while got.len() < 16 && got.last() != Some(&b'\n') {
                match stream.read(&mut byte) {
                    Ok(1) => got.push(byte[0]),
                    _ => break,
                }
            }
            if got == b"BPB1 OK\n" {
                return Some(Peer::direct(stream));
            }
        }
    }
    None
}

fn find_room(code: &str, paths: Paths) -> Result<Peer, String> {
    if paths.lan {
        if let Some(peer) = find_on_lan(code) {
            return Ok(peer);
        }
    }
    if !paths.relay {
        return Err(format!("No room {code} on this network"));
    }
    let me = random_id();
    let index = relay_for(code);
    let will = (topic(code, "h"), format!("{me} BYE"));
    let mut mqtt = Mqtt::connect(index, (&will.0, &will.1), &topic(code, "g"))?;
    let hello = format!("{me} HELLO");
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut last_hello = Instant::now() - Duration::from_secs(10);
    while Instant::now() < deadline {
        if last_hello.elapsed() > Duration::from_millis(1500) {
            mqtt.publish(&topic(code, "h"), &hello);
            last_hello = Instant::now();
        }
        let payloads = mqtt.poll();
        for (i, payload) in payloads.iter().enumerate() {
            let mut words = payload.split_whitespace();
            let (host, word, guest) = (words.next(), words.next(), words.next());
            if let (Some(host), Some("WELCOME"), Some(g)) = (host, word, guest) {
                if g == me {
                    // The host starts talking straight after the welcome;
                    // keep anything that arrived with it.
                    mqtt.backlog = payloads[i + 1..].to_vec();
                    return Ok(Peer::with(Link::Relay {
                        mqtt,
                        out: topic(code, "h"),
                        me,
                        them: host.to_string(),
                    }));
                }
            }
        }
        if !mqtt.alive {
            return Err("Lost the connection to the online relay".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    mqtt.disconnect();
    Err(format!("No room {code} found - check the code with your friend"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Msg> {
        vec![
            Msg::Start {
                mode: 1,
                seed: 0xDEADBEEF12345678,
                first: 1,
            },
            Msg::Cue {
                x: -0.635,
                y: 0.1,
                aim: 3.1,
                power: 0.5,
                side: -0.25,
                top: 1.0,
            },
            Msg::Shot {
                x: 0.123456789,
                y: -0.2,
                angle: 1.0000000001,
                speed: 8.6,
                side: 0.0,
                top: -0.6,
                call: ShotCall {
                    ball: Some(8),
                    pocket: Some(5),
                    safety: false,
                    push_out: true,
                },
            },
            Msg::Shot {
                x: 0.0,
                y: 0.0,
                angle: 0.0,
                speed: 1.0,
                side: 0.0,
                top: 0.0,
                call: ShotCall {
                    safety: true,
                    ..ShotCall::default()
                },
            },
            Msg::State(
                vec![(0, true, 0.1, 0.2), (8, false, 1.29, 0.655)],
                ShotLog {
                    first_hit: Some(3),
                    potted: vec![8, 3],
                    pockets: vec![2, 5],
                    cushion_after_contact: true,
                    balls_to_cushion: 4,
                    crossed_head_string: true,
                    ..ShotLog::default()
                },
            ),
            Msg::Choice(3),
        ]
    }

    #[test]
    fn messages_survive_encoding_exactly() {
        for m in samples() {
            assert_eq!(Msg::decode(&m.encode()), Some(m));
        }
        assert_eq!(Msg::decode("garbage 1 2 3"), None);
        assert_eq!(Msg::decode("SHOT 1 2"), None);
    }

    #[test]
    fn room_codes_are_valid_and_pick_their_relay() {
        for relay in 0..RELAYS.len() {
            for _ in 0..50 {
                let code = random_code(relay);
                assert_eq!(code.len(), CODE_LEN);
                assert_eq!(normalize_code(&code).as_deref(), Some(code.as_str()));
                assert_eq!(relay_for(&code), relay);
            }
        }
        assert_eq!(normalize_code(" k7q-x2m ").as_deref(), Some("K7QX2M"));
        assert_eq!(normalize_code("K7QX2"), None);
        assert_eq!(normalize_code("K7QX2O"), None, "O is not used in codes");
        assert_ne!(random_code(0), random_code(0));
    }

    #[test]
    fn mqtt_packets_split_correctly() {
        let small = packet(0x30, b"hello");
        let big_body = vec![b'x'; 300];
        let big = packet(0x30, &big_body);
        assert_eq!(big[1], 0x80 | (300 % 128) as u8);
        let mut buf = [small.clone(), big.clone()].concat();
        assert_eq!(take_packet(&mut buf), Some((0x30, b"hello".to_vec())));
        // A half-arrived packet waits for the rest.
        let mut partial = big[..100].to_vec();
        assert_eq!(take_packet(&mut partial), None);
        assert_eq!(take_packet(&mut buf), Some((0x30, big_body)));
        assert!(buf.is_empty());
    }

    fn exchange(mut a: Peer, mut b: Peer) {
        for m in samples() {
            a.send(&m);
            b.send(&m);
        }
        let collect = |p: &mut Peer| {
            let mut got = Vec::new();
            for _ in 0..1000 {
                got.extend(p.poll());
                if got.len() >= samples().len() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            got
        };
        assert_eq!(collect(&mut b), samples());
        assert_eq!(collect(&mut a), samples());
        drop(a);
        for _ in 0..1000 {
            b.poll();
            if !b.alive {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!b.alive, "the other side should notice a player leaving");
    }

    /// Only one room per computer can listen for LAN searches, so tests that
    /// open rooms take turns.
    static ROOM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn pair(paths: Paths) -> (Peer, Peer) {
        let _turn = ROOM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut room = create_room(paths).recv_timeout(Duration::from_secs(15)).unwrap().unwrap();
        let rx = join_room(&room.code.to_lowercase(), paths);
        let mut host = None;
        let mut guest = None;
        for _ in 0..2000 {
            if host.is_none() {
                host = room.poll();
                // Like the game, the host talks the moment the guest arrives,
                // before the guest's side of the handshake has finished.
                if let Some(h) = host.as_mut() {
                    h.send(&samples()[0]);
                }
            }
            if guest.is_none() {
                if let Ok(r) = rx.try_recv() {
                    guest = Some(r.unwrap());
                }
            }
            if host.is_some() && guest.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let host = host.expect("host never saw the guest");
        let mut guest = guest.expect("guest never joined");
        let mut first = Vec::new();
        for _ in 0..1000 {
            first.extend(guest.poll());
            if !first.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(first, vec![samples()[0].clone()], "the host's first message was lost in the handshake");
        (host, guest)
    }

    #[test]
    fn players_meet_on_the_local_network_by_code() {
        let (host, guest) = pair(Paths { lan: true, relay: false });
        assert!(!host.relayed() && !guest.relayed());
        exchange(host, guest);
    }

    #[test]
    fn lost_messages_are_resent_in_order() {
        let (mut host, mut guest) = pair(Paths { lan: true, relay: false });
        // The first two numbered messages vanish on the way.
        host.drop_next = 2;
        for m in samples() {
            host.send(&m);
        }
        let mut got = Vec::new();
        for _ in 0..2000 {
            host.poll();
            got.extend(guest.poll());
            if got.len() >= samples().len() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Live aiming updates are not re-sent, so the lost Cue is gone; the
        // rest arrives complete, in order and without duplicates.
        let expected: Vec<Msg> = samples().into_iter().filter(|m| !matches!(m, Msg::Cue { .. })).collect();
        let got: Vec<Msg> = got.into_iter().filter(|m| !matches!(m, Msg::Cue { .. })).collect();
        assert_eq!(got, expected);
        for _ in 0..50 {
            host.poll();
            guest.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(host.unacked.is_empty(), "everything should be acknowledged");
    }

    #[test]
    fn a_wrong_code_is_rejected_quickly() {
        let r = join_room("ZZZZZZ", Paths { lan: true, relay: false })
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(r.is_err());
        assert!(join_room("nope", Paths::all()).recv_timeout(Duration::from_secs(2)).unwrap().is_err());
    }

    /// Uses the real public relay, so it needs the internet:
    /// `cargo test --release relay -- --ignored`
    #[test]
    #[ignore]
    fn players_meet_through_the_internet_relay() {
        let (host, guest) = pair(Paths { lan: false, relay: true });
        assert!(host.relayed() && guest.relayed());
        exchange(host, guest);
    }
}
