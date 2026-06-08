//! Bridge between serde_json::Value and yrs::Any, so the server can read/write
//! sticky/frame objects the same shape the JS yjs client uses.
use std::collections::HashMap;
use std::sync::Arc;
use yrs::Any;

pub fn json_to_any(v: &serde_json::Value) -> Any {
    match v {
        serde_json::Value::Null => Any::Null,
        serde_json::Value::Bool(b) => Any::Bool(*b),
        serde_json::Value::Number(n) => Any::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Any::String(Arc::from(s.as_str())),
        serde_json::Value::Array(a) => {
            Any::Array(Arc::from(a.iter().map(json_to_any).collect::<Vec<_>>()))
        }
        serde_json::Value::Object(o) => {
            let m: HashMap<String, Any> = o
                .iter()
                .map(|(k, val)| (k.clone(), json_to_any(val)))
                .collect();
            Any::Map(Arc::new(m))
        }
    }
}

pub fn any_to_json(a: &Any) -> serde_json::Value {
    use serde_json::Value as J;
    match a {
        Any::Null | Any::Undefined => J::Null,
        Any::Bool(b) => J::Bool(*b),
        Any::Number(n) => serde_json::Number::from_f64(*n)
            .map(J::Number)
            .unwrap_or(J::Null),
        Any::BigInt(i) => J::Number((*i).into()),
        Any::String(s) => J::String(s.to_string()),
        Any::Buffer(_) => J::Null,
        Any::Array(arr) => J::Array(arr.iter().map(any_to_json).collect()),
        Any::Map(m) => {
            let mut o = serde_json::Map::new();
            for (k, v) in m.iter() {
                o.insert(k.clone(), any_to_json(v));
            }
            J::Object(o)
        }
    }
}

/// Read a yrs Map (e.g. "shapes") into a Vec of JSON objects.
pub fn map_values_json(txn: &impl yrs::ReadTxn, map: &yrs::MapRef) -> Vec<serde_json::Value> {
    use yrs::types::ToJson;
    use yrs::Map;
    map.iter(txn)
        .map(|(_k, v)| any_to_json(&v.to_json(txn)))
        .collect()
}
