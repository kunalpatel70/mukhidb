use crate::types::{Column, DataType, Row, Value};

const TEXT_MAX: usize = 252;
const TEXT_SLOT: usize = 4 + TEXT_MAX; // 256 bytes per TEXT column

/// Byte size of a single column value on disk.
fn col_size(dt: &DataType) -> usize {
    match dt {
        DataType::Integer => 8,
        DataType::Text => TEXT_SLOT,
    }
}

/// Total byte size of a serialized row for the given schema.
pub fn row_size(columns: &[Column]) -> usize {
    columns.iter().map(|c| col_size(&c.data_type)).sum()
}

/// Serialize a Row into buf (must be at least row_size bytes).
pub fn serialize(row: &Row, columns: &[Column], buf: &mut [u8]) {
    let mut offset = 0;
    for (col, val) in columns.iter().zip(row.values.iter()) {
        match (&col.data_type, val) {
            (DataType::Integer, Value::Integer(n)) => {
                buf[offset..offset + 8].copy_from_slice(&n.to_le_bytes());
                offset += 8;
            }
            (DataType::Text, Value::Text(s)) => {
                let bytes = s.as_bytes();
                let len = bytes.len().min(TEXT_MAX);
                buf[offset..offset + 4].copy_from_slice(&(len as u32).to_le_bytes());
                buf[offset + 4..offset + 4 + len].copy_from_slice(&bytes[..len]);
                // zero-fill remainder
                for b in &mut buf[offset + 4 + len..offset + TEXT_SLOT] {
                    *b = 0;
                }
                offset += TEXT_SLOT;
            }
            _ => {
                // zero-fill for type mismatch / Null
                let sz = col_size(&col.data_type);
                for b in &mut buf[offset..offset + sz] {
                    *b = 0;
                }
                offset += sz;
            }
        }
    }
}

/// Deserialize a Row from buf using the given schema.
pub fn deserialize(buf: &[u8], columns: &[Column]) -> Row {
    let mut values = Vec::with_capacity(columns.len());
    let mut offset = 0;
    for col in columns {
        match col.data_type {
            DataType::Integer => {
                let n = i64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
                values.push(Value::Integer(n));
                offset += 8;
            }
            DataType::Text => {
                let len = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
                let len = len.min(TEXT_MAX);
                let s = String::from_utf8_lossy(&buf[offset + 4..offset + 4 + len]).to_string();
                values.push(Value::Text(s));
                offset += TEXT_SLOT;
            }
        }
    }
    Row { values }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let cols = vec![
            Column { name: "id".into(), data_type: DataType::Integer },
            Column { name: "name".into(), data_type: DataType::Text },
        ];
        let row = Row {
            values: vec![Value::Integer(42), Value::Text("Alice".into())],
        };
        let size = row_size(&cols);
        let mut buf = vec![0u8; size];
        serialize(&row, &cols, &mut buf);
        let out = deserialize(&buf, &cols);
        assert_eq!(out.values[0], Value::Integer(42));
        assert_eq!(out.values[1], Value::Text("Alice".into()));
    }
}
