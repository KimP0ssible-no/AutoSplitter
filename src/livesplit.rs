//! LiveSplit Server client: text commands over TCP, one per CRLF-terminated line.

use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

pub struct LiveSplit {
    host: String,
    port: u16,
    stream: Option<TcpStream>,
}

impl LiveSplit {
    pub fn connect(host: &str, port: u16) -> std::io::Result<LiveSplit> {
        let mut ls = LiveSplit { host: host.into(), port, stream: None };
        ls.stream = Some(ls.open()?);
        Ok(ls)
    }

    fn open(&self) -> std::io::Result<TcpStream> {
        let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no address");
        for addr in (self.host.as_str(), self.port).to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, TIMEOUT) {
                Ok(s) => {
                    s.set_write_timeout(Some(TIMEOUT))?;
                    return Ok(s);
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Sends one command. A failed send reconnects once and sends again, because LiveSplit drops idle clients when it restarts its server.
    pub fn send(&mut self, command: &str) -> std::io::Result<()> {
        let line = format!("{command}\r\n");
        if let Some(s) = &mut self.stream
            && s.write_all(line.as_bytes()).is_ok()
        {
            return Ok(());
        }
        self.stream = None;
        let mut s = self.open()?;
        s.write_all(line.as_bytes())?;
        self.stream = Some(s);
        Ok(())
    }

    pub fn start(&mut self) -> std::io::Result<()> {
        self.send("starttimer")?;
        self.send("initgametime")
    }

    /// Splits at `total` seconds of game time, after one skipsplit per skipped segment.
    pub fn split(&mut self, skipped: usize, total: f64) -> std::io::Result<()> {
        for _ in 0..skipped {
            self.send("skipsplit")?;
        }
        self.send(&game_time(total))?;
        self.send("split")
    }

    pub fn reset(&mut self) -> std::io::Result<()> {
        self.send("reset")
    }
}

/// "setgametime H:MM:SS.fffffff", the .NET TimeSpan form LiveSplit parses.
fn game_time(seconds: f64) -> String {
    let ticks = (seconds.max(0.0) * 1e7) as u64;
    let s = ticks / 10_000_000;
    format!("setgametime {}:{:02}:{:02}.{:07}", s / 3600, s / 60 % 60, s % 60, ticks % 10_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn sends_crlf_commands() {
        assert_eq!(game_time(3725.5), "setgametime 1:02:05.5000000");
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let mut ls = LiveSplit::connect("127.0.0.1", port).unwrap();
        let (mut peer, _) = server.accept().unwrap();
        ls.start().unwrap();
        ls.split(1, 10.0).unwrap();
        drop(ls);
        let mut got = String::new();
        peer.read_to_string(&mut got).unwrap();
        assert_eq!(got, "starttimer\r\ninitgametime\r\nskipsplit\r\nsetgametime 0:00:10.0000000\r\nsplit\r\n");
    }
}
