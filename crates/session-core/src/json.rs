//! JSON values with a fixed key order.

use serde_json::Value;

/// `value` with the keys of each object in sorted order, at each level.
///
/// A dependency can enable the `preserve_order` feature of `serde_json`.
/// Cargo enables a feature for the whole build, so each `serde_json::Map`
/// then keeps the order of insertion. Serialize the result of this function
/// when the bytes must not change between builds, for example bytes that a
/// digest is made from or bytes of a file that Longhand writes. The bytes are
/// the same as the bytes of a build without the feature.
pub fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_of_each_object_are_in_sorted_order() {
        let value: Value =
            serde_json::from_str(r#"{"b":1,"a":[{"d":2,"c":3}],"c":{"z":0,"y":1}}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&sorted(value)).unwrap(),
            r#"{"a":[{"c":3,"d":2}],"b":1,"c":{"y":1,"z":0}}"#
        );
    }
}
