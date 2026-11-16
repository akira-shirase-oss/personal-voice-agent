//! Bounded JSON-lines framing on the app's stdin. A line longer than the limit is reported
//! and discarded up to its newline, so a runaway writer cannot grow the service's memory.

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;
use voice_domain::protocol::ClientMessage;

pub const MAX_LINE: usize = 64 * 1024;

#[derive(Debug)]
pub enum Inbound {
    Message(ClientMessage),
    /// A malformed or oversized line: an error code, never the line's content.
    Bad(&'static str),
    Eof,
    ReadError,
}

enum Read {
    Line,
    TooLong,
    Eof,
}

async fn read_line<R: AsyncRead + Unpin>(
    r: &mut BufReader<R>,
    line: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<Read> {
    let mut discarding = false;
    loop {
        let buf = r.fill_buf().await?;
        if buf.is_empty() {
            // EOF: a final line without a newline still counts as a line.
            return Ok(if discarding {
                Read::TooLong
            } else if line.is_empty() {
                Read::Eof
            } else {
                Read::Line
            });
        }
        let newline = buf.iter().position(|&b| b == b'\n');
        let take = newline.unwrap_or(buf.len());
        if !discarding {
            if line.len() + take > max {
                discarding = true;
                line.clear();
            } else {
                line.extend_from_slice(&buf[..take]);
            }
        }
        let consumed = newline.map_or(take, |i| i + 1);
        r.consume(consumed);
        if newline.is_some() {
            return Ok(if discarding { Read::TooLong } else { Read::Line });
        }
    }
}

/// Reads the app's lines until EOF, sending each parsed message on `tx` (bounded: a slow
/// session applies backpressure to stdin instead of buffering without limit).
pub async fn read_loop<R: AsyncRead + Unpin>(reader: R, tx: mpsc::Sender<Inbound>, max: usize) {
    let mut r = BufReader::with_capacity(64 * 1024, reader);
    let mut line = Vec::with_capacity(1_024);
    loop {
        line.clear();
        let item = match read_line(&mut r, &mut line, max).await {
            Ok(Read::Line) => {
                let trimmed = line.strip_suffix(b"\r").unwrap_or(&line);
                if trimmed.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                match serde_json::from_slice::<ClientMessage>(trimmed) {
                    Ok(m) => Inbound::Message(m),
                    Err(_) => Inbound::Bad("bad_message"),
                }
            }
            Ok(Read::TooLong) => Inbound::Bad("line_too_long"),
            Ok(Read::Eof) => {
                let _ = tx.send(Inbound::Eof).await;
                return;
            }
            Err(_) => {
                let _ = tx.send(Inbound::ReadError).await;
                return;
            }
        };
        if tx.send(item).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn collect(input: &'static [u8], max: usize) -> Vec<String> {
        let (tx, mut rx) = mpsc::channel(16);
        read_loop(input, tx, max).await;
        let mut out = Vec::new();
        while let Some(i) = rx.recv().await {
            out.push(match i {
                Inbound::Message(m) => m.name().to_owned(),
                Inbound::Bad(code) => code.to_owned(),
                Inbound::Eof => "eof".to_owned(),
                Inbound::ReadError => "read_error".to_owned(),
            });
        }
        out
    }

    #[tokio::test]
    async fn lines_are_parsed_bounded_and_recovered_after_garbage() {
        let input = b"{\"type\":\"ptt_down\"}\nnot json\n\n{\"type\":\"interrupt\"}\r\n{\"type\":\"flush\"}";
        assert_eq!(
            collect(input, 1_024).await,
            ["ptt_down", "bad_message", "interrupt", "flush", "eof"]
        );
    }

    #[tokio::test]
    async fn an_oversized_line_is_discarded_and_the_next_one_still_reads() {
        let mut input = vec![b'x'; 5_000];
        input.push(b'\n');
        input.extend_from_slice(b"{\"type\":\"ptt_down\"}\n");
        let input: &'static [u8] = Box::leak(input.into_boxed_slice());
        assert_eq!(collect(input, 100).await, ["line_too_long", "ptt_down", "eof"]);
    }
}
