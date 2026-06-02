use serde_json::{json, Value, Map};
use sha2::{Sha256, Digest};

const VOLATILE_TOP_LEVEL: &[&str] = &["metadata"];
const KEY_CONTEXT_FIELD: &str = "__key_context__";

pub fn normalize_payload(payload: &Value) -> Value {
    if !payload.is_object() {
        return json!({
            "__non_dict_payload__": true,
            "repr": format!("{:?}", payload)
        });
    }

    let mut out = Map::new();
    if let Some(obj) = payload.as_object() {
        for (k, v) in obj {
            if VOLATILE_TOP_LEVEL.contains(&k.as_str()) {
                continue;
            }
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

pub fn canonical_text(payload: &Value, key_context: Option<&Value>) -> String {
    let normalized = normalize_payload(payload);
    let mut final_val = normalized;

    if let Some(ctx) = key_context {
        if let Some(obj) = final_val.as_object_mut() {
            obj.insert(KEY_CONTEXT_FIELD.to_string(), ctx.clone());
        }
    }

    serde_json::to_string(&final_val).unwrap_or_default()
}

pub fn cache_key(payload: &Value, key_context: Option<&Value>) -> String {
    let encoded = canonical_text(payload, key_context);
    let mut hasher = Sha256::new();
    hasher.update(encoded.as_bytes());
    format!("{:x}", hasher.finalize())
}
