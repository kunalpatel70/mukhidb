use crate::types::{Column, DataType, Row, Value};

/// Compute the serialized byte size of a specific row.
pub fn serialized_size(row: &Row, columns: &[Column]) -> usize {
    let mut size = 0;
    for (col, val) in columns.iter().zip(row.values.iter()) {
        size += match (&col.data_type, val) {
            (DataType::Integer, _) => 8,
            (DataType::Text, Value::Text(s)) => 4 + s.len(),
            (DataType::Text, _) => 4, // empty text for type mismatch / Null
        };
    }
    size
}

/// Serialize a Row into buf (must be at least serialized_size bytes).
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
                let len = bytes.len();
                buf[offset..offset + 4].copy_from_slice(&(len as u32).to_le_bytes());
                offset += 4;
                buf[offset..offset + len].copy_from_slice(bytes);
                offset += len;
            }
            _ => {
                // Zero-fill for type mismatch / Null.
                let sz = match col.data_type {
                    DataType::Integer => 8,
                    DataType::Text => 4, // just a zero-length prefix
                };
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
                let len =
                    u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
                offset += 4;
                let s = String::from_utf8_lossy(&buf[offset..offset + len]).to_string();
                values.push(Value::Text(s));
                offset += len;
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
        let size = serialized_size(&row, &cols);
        assert_eq!(size, 8 + 4 + 5); // i64 + len_prefix + "Alice"
        let mut buf = vec![0u8; size];
        serialize(&row, &cols, &mut buf);
        let out = deserialize(&buf, &cols);
        assert_eq!(out.values[0], Value::Integer(42));
        assert_eq!(out.values[1], Value::Text("Alice".into()));
    }

    #[test]
    fn variable_sizes() {
        let cols = vec![
            Column { name: "id".into(), data_type: DataType::Integer },
            Column { name: "bio".into(), data_type: DataType::Text },
        ];
        // Short text
        let short = Row {
            values: vec![Value::Integer(1), Value::Text("Hi".into())],
        };
        assert_eq!(serialized_size(&short, &cols), 8 + 4 + 2);

        // Long text
        let long_text = "x".repeat(1000);
        let long = Row {
            values: vec![Value::Integer(2), Value::Text(long_text.clone())],
        };
        assert_eq!(serialized_size(&long, &cols), 8 + 4 + 1000);

        // Round-trip both
        for row in [&short, &long] {
            let size = serialized_size(row, &cols);
            let mut buf = vec![0u8; size];
            serialize(row, &cols, &mut buf);
            let out = deserialize(&buf, &cols);
            assert_eq!(out.values, row.values);
        }
    }

    #[test]
    fn empty_text() {
        let cols = vec![
            Column { name: "id".into(), data_type: DataType::Integer },
            Column { name: "name".into(), data_type: DataType::Text },
        ];
        let row = Row {
            values: vec![Value::Integer(1), Value::Text(String::new())],
        };
        let size = serialized_size(&row, &cols);
        assert_eq!(size, 8 + 4); // i64 + len_prefix(0)
        let mut buf = vec![0u8; size];
        serialize(&row, &cols, &mut buf);
        let out = deserialize(&buf, &cols);
        assert_eq!(out.values[1], Value::Text(String::new()));
    }
}