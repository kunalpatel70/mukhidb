/// Wire protocol for mukhidb client-server communication.
///
/// Every message is length-prefixed and has a type tag:
///   [length: u32 LE (4 bytes)] [type: u8 (1 byte)] [payload: length-1 bytes]
///
/// Client -> Server:
///   0x01 Query   — payload is UTF-8 SQL
///
/// Server -> Client:
///   0x02 Ok      — payload is UTF-8 success message
///   0x03 Error   — payload is UTF-8 error text
///   0x04 Rows    — payload is serialized results:
///     [num_cols: u32][col_name: u32 len + bytes]...
///     [num_rows: u32][for each row: [cell: u32 len + bytes]...]

use std::io::{Read, Write};

pub const MSG_QUERY: u8 = 0x01;
pub const MSG_OK: u8 = 0x02;
pub const MSG_ERROR: u8 = 0x03;
pub const MSG_ROWS: u8 = 0x04;

/// A message sent over the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Query(String),
    Ok(String),
    Error(String),
    Rows {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

/// Read exactly n bytes from a reader, returning an error on EOF or short read.
fn read_exact_bytes<R: Read>(r: &mut R, n: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Read one framed message from a reader.
pub fn read_message<R: Read>(r: &mut R) -> std::io::Result<Message> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    if len < 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Frame length must be at least 1 (type byte)",
        ));
    }

    let mut type_buf = [0u8; 1];
    r.read_exact(&mut type_buf)?;
    let msg_type = type_buf[0];

    let payload = read_exact_bytes(r, len - 1)?;

    match msg_type {
        MSG_QUERY => Ok(Message::Query(String::from_utf8_lossy(&payload).to_string())),
        MSG_OK => Ok(Message::Ok(String::from_utf8_lossy(&payload).to_string())),
        MSG_ERROR => Ok(Message::Error(String::from_utf8_lossy(&payload).to_string())),
        MSG_ROWS => decode_rows(&payload).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        }),
        t => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Unknown message type: 0x{:02X}", t),
        )),
    }
}

/// Write one framed message to a writer.
pub fn write_message<W: Write>(w: &mut W, msg: &Message) -> std::io::Result<()> {
    let (msg_type, payload) = match msg {
        Message::Query(s) => (MSG_QUERY, s.as_bytes().to_vec()),
        Message::Ok(s) => (MSG_OK, s.as_bytes().to_vec()),
        Message::Error(s) => (MSG_ERROR, s.as_bytes().to_vec()),
        Message::Rows { headers, rows } => (MSG_ROWS, encode_rows(headers, rows)),
    };

    let len = (payload.len() + 1) as u32;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&[msg_type])?;
    w.write_all(&payload)?;
    w.flush()?;
    Ok(())
}

/// Encode a Rows payload.
fn encode_rows(headers: &[String], rows: &[Vec<String>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(headers.len() as u32).to_le_bytes());
    for h in headers {
        let b = h.as_bytes();
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        out.extend_from_slice(b);
    }
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for row in rows {
        for cell in row {
            let b = cell.as_bytes();
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(b);
        }
    }
    out
}

/// Decode a Rows payload.
fn decode_rows(buf: &[u8]) -> Result<Message, String> {
    let mut off = 0;
    let read_u32 = |off: &mut usize, buf: &[u8]| -> Result<u32, String> {
        if *off + 4 > buf.len() {
            return Err("Truncated Rows payload (u32)".to_string());
        }
        let v = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap());
        *off += 4;
        Ok(v)
    };
    let read_str = |off: &mut usize, buf: &[u8]| -> Result<String, String> {
        let len = read_u32(off, buf)? as usize;
        if *off + len > buf.len() {
            return Err("Truncated Rows payload (string)".to_string());
        }
        let s = String::from_utf8_lossy(&buf[*off..*off + len]).to_string();
        *off += len;
        Ok(s)
    };

    let ncols = read_u32(&mut off, buf)? as usize;
    let mut headers = Vec::with_capacity(ncols);
    for _ in 0..ncols {
        headers.push(read_str(&mut off, buf)?);
    }

    let nrows = read_u32(&mut off, buf)? as usize;
    let mut rows = Vec::with_capacity(nrows);
    for _ in 0..nrows {
        let mut row = Vec::with_capacity(ncols);
        for _ in 0..ncols {
            row.push(read_str(&mut off, buf)?);
        }
        rows.push(row);
    }

    Ok(Message::Rows { headers, rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn round_trip(msg: Message) -> Message {
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();
        let mut cursor = Cursor::new(buf);
        read_message(&mut cursor).unwrap()
    }

    #[test]
    fn query_round_trip() {
        let m = Message::Query("SELECT * FROM users".to_string());
        assert_eq!(round_trip(m.clone()), m);
    }

    #[test]
    fn ok_round_trip() {
        let m = Message::Ok("1 row inserted.".to_string());
        assert_eq!(round_trip(m.clone()), m);
    }

    #[test]
    fn error_round_trip() {
        let m = Message::Error("Table not found.".to_string());
        assert_eq!(round_trip(m.clone()), m);
    }

    #[test]
    fn rows_round_trip() {
        let m = Message::Rows {
            headers: vec!["id".into(), "name".into()],
            rows: vec![
                vec!["1".into(), "Alice".into()],
                vec!["2".into(), "Bob".into()],
            ],
        };
        assert_eq!(round_trip(m.clone()), m);
    }

    #[test]
    fn empty_rows_round_trip() {
        let m = Message::Rows {
            headers: vec!["id".into()],
            rows: vec![],
        };
        assert_eq!(round_trip(m.clone()), m);
    }

    #[test]
    fn unicode_payload() {
        let m = Message::Query("SELECT * FROM users WHERE name = '日本語'".to_string());
        assert_eq!(round_trip(m.clone()), m);
    }

    #[test]
    fn multiple_messages_in_stream() {
        let mut buf = Vec::new();
        write_message(&mut buf, &Message::Query("q1".into())).unwrap();
        write_message(&mut buf, &Message::Query("q2".into())).unwrap();
        write_message(&mut buf, &Message::Ok("done".into())).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(read_message(&mut cursor).unwrap(), Message::Query("q1".into()));
        assert_eq!(read_message(&mut cursor).unwrap(), Message::Query("q2".into()));
        assert_eq!(read_message(&mut cursor).unwrap(), Message::Ok("done".into()));
    }

    #[test]
    fn unknown_type_tag_errors() {
        let buf = vec![0x01, 0x00, 0x00, 0x00, 0xFF]; // len=1, type=0xFF
        let mut cursor = Cursor::new(buf);
        assert!(read_message(&mut cursor).is_err());
    }
}