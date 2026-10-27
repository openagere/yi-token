//! Byte-buffered SSE framing.
//!
//! Chunks may split a multibyte UTF-8 character anywhere, so bytes are buffered
//! until a whole line is available and only then decoded.

use bytes::Bytes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl Decoder {
    /// Feed raw bytes; returns every frame completed by this chunk.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        self.buf.extend_from_slice(chunk);
        let mut frames = Vec::new();
        let mut start = 0;
        let mut i = 0;
        while i < self.buf.len() {
            let b = self.buf[i];
            if b != b'\n' && b != b'\r' {
                i += 1;
                continue;
            }
            let mut next = i + 1;
            if b == b'\r' {
                match self.buf.get(next) {
                    // A trailing CR may be the first half of CRLF; wait for more bytes.
                    None => break,
                    Some(b'\n') => next += 1,
                    Some(_) => {}
                }
            }
            let line = String::from_utf8_lossy(&self.buf[start..i]).into_owned();
            if let Some(frame) = self.line(&line) {
                frames.push(frame);
            }
            start = next;
            i = next;
        }
        self.buf.drain(..start);
        frames
    }

    fn line(&mut self, line: &str) -> Option<SseFrame> {
        if line.is_empty() {
            let event = self.event.take();
            if self.data.is_empty() {
                return None;
            }
            return Some(SseFrame {
                event,
                data: std::mem::take(&mut self.data).join("\n"),
            });
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
        None
    }
}

pub fn encode(event: Option<&str>, data: &str) -> Bytes {
    let mut out = String::new();
    if let Some(event) = event {
        out.push_str("event: ");
        out.push_str(event);
        out.push('\n');
    }
    for line in data.split('\n') {
        out.push_str("data: ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    Bytes::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_decoder_handles_multibyte_split() {
        let bytes = "data: 你好\n\n".as_bytes();
        // "data: " is 6 bytes; cut after the 2nd byte of 你.
        let cut = 8;
        let mut d = Decoder::default();
        assert!(d.push(&bytes[..cut]).is_empty());
        let frames = d.push(&bytes[cut..]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "你好");
        assert!(!frames[0].data.contains('\u{FFFD}'));
    }

    #[test]
    fn sse_decoder_handles_every_split_point() {
        let bytes = "event: x\ndata: 你好🙂\n\n".as_bytes();
        for cut in 0..=bytes.len() {
            let mut d = Decoder::default();
            let mut frames = d.push(&bytes[..cut]);
            frames.extend(d.push(&bytes[cut..]));
            assert_eq!(frames.len(), 1, "cut {cut}");
            assert_eq!(frames[0].data, "你好🙂");
            assert_eq!(frames[0].event.as_deref(), Some("x"));
        }
    }

    #[test]
    fn sse_decoder_handles_crlf_and_comments() {
        let mut d = Decoder::default();
        let mut frames = d.push(b": keep-alive\r\nevent: a\r\ndata: 1\r\ndata: 2\r\n\r");
        frames.extend(d.push(b"\ndata:3\rdata:4\r\n\r\n"));
        assert_eq!(
            frames,
            vec![
                SseFrame {
                    event: Some("a".into()),
                    data: "1\n2".into()
                },
                SseFrame {
                    event: None,
                    data: "3\n4".into()
                },
            ]
        );
    }

    #[test]
    fn encode_frames() {
        assert_eq!(
            &encode(Some("e"), "a\nb")[..],
            b"event: e\ndata: a\ndata: b\n\n"
        );
        assert_eq!(&encode(None, "x")[..], b"data: x\n\n");
    }
}
