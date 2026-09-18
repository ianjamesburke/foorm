//! OSC intake: one thread reads UDP, decodes packets, and turns the nooise
//! address vocabulary into typed events. Unknown addresses are counted, not
//! dropped silently, so the settings overlay can show what is arriving.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use rosc::{OscPacket, OscType, decoder};

/// One decoded message from a producer.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Beat(f32),
    /// Master output RMS; zero is silence whatever the tempo does.
    Level(f32),
    /// Chord index plus the attack and release seconds of the pad layer it
    /// voiced, so colour can move at the speed the sound does.
    Chord {
        index: i32,
        attack: f32,
        release: f32,
    },
    /// One kick hit at this `kick.level` (0 = inaudible).
    Kick(f32),
    /// Any address foorm does not map to a shape yet.
    Unknown(String),
}

pub fn listen(addr: SocketAddr) -> io::Result<Receiver<Event>> {
    let socket = UdpSocket::bind(addr)
        .map_err(|e| io::Error::new(e.kind(), format!("osc: bind {addr} failed: {e}")))?;
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("foorm-osc".into())
        .spawn(move || receive_loop(&socket, &tx))?;
    Ok(rx)
}

fn receive_loop(socket: &UdpSocket, tx: &Sender<Event>) {
    let mut buf = [0u8; rosc::decoder::MTU];
    loop {
        let Ok((len, _)) = socket.recv_from(&mut buf) else {
            continue;
        };
        let Ok((_, packet)) = decoder::decode_udp(&buf[..len]) else {
            continue;
        };
        for event in events_in(packet) {
            if tx.send(event).is_err() {
                return; // UI gone; nothing left to feed
            }
        }
    }
}

fn events_in(packet: OscPacket) -> Vec<Event> {
    match packet {
        OscPacket::Message(m) => vec![event_for(&m.addr, &m.args)],
        OscPacket::Bundle(b) => b.content.into_iter().flat_map(events_in).collect(),
    }
}

fn event_for(addr: &str, args: &[OscType]) -> Event {
    match (addr, args) {
        ("/nooise/beat", [OscType::Float(b)]) => Event::Beat(*b),
        ("/nooise/level", [OscType::Float(l)]) => Event::Level(*l),
        ("/nooise/chord", [OscType::Int(c), OscType::Float(a), OscType::Float(r)]) => {
            Event::Chord {
                index: *c,
                attack: *a,
                release: *r,
            }
        }
        ("/nooise/voice/kick", [OscType::Float(l)]) => Event::Kick(*l),
        _ => Event::Unknown(addr.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{OscBundle, OscMessage, OscTime, encoder};

    #[test]
    fn decodes_nooise_vocabulary_from_a_bundle() {
        let bundle = OscPacket::Bundle(OscBundle {
            timetag: OscTime::from((0, 1)),
            content: vec![
                OscPacket::Message(OscMessage {
                    addr: "/nooise/beat".into(),
                    args: vec![OscType::Float(3.25)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/nooise/level".into(),
                    args: vec![OscType::Float(0.2)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/nooise/chord".into(),
                    args: vec![OscType::Int(2), OscType::Float(6.0), OscType::Float(8.0)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/nooise/voice/kick".into(),
                    args: vec![OscType::Float(0.8)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/other".into(),
                    args: vec![],
                }),
            ],
        });
        let bytes = encoder::encode(&bundle).unwrap();
        let (_, packet) = decoder::decode_udp(&bytes).unwrap();
        assert_eq!(
            events_in(packet),
            [
                Event::Beat(3.25),
                Event::Level(0.2),
                Event::Chord {
                    index: 2,
                    attack: 6.0,
                    release: 8.0
                },
                Event::Kick(0.8),
                Event::Unknown("/other".into())
            ]
        );
    }
}
