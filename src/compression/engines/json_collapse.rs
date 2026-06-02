use crate::compression::engines::base::{EngineResult, make_result, identity_result};
use serde_json::{Value, Map};
use std::collections::BTreeMap;

const OMITTED_KEY: &str = "__middleout_omitted__";

fn level_config(level: &str) -> (usize, Option<usize>) {
    match level {
        "lite" => (50, None),
        "standard" => (20, Some(50)),
        "aggressive" => (10, Some(20)),
        _ => (20, Some(50)),
    }
}

fn type_name(val: &Value) -> &'static str {
    match val {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(num) => {
            if num.is_f64() { "float" } else { "int" }
        }
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

fn type_counts(items: &[Value]) -> String {
    let mut counts = BTreeMap::new();
    for val in items {
        let name = type_name(val);
        *counts.entry(name).or_insert(0) += 1;
    }
    let parts: Vec<String> = counts.iter().map(|(t, n)| format!("{}={}", t, n)).collect();
    parts.join(", ")
}

struct Collapser {
    arr_th: usize,
    obj_th: Option<usize>,
    collapses: usize,
}

impl Collapser {
    fn new(arr_th: usize, obj_th: Option<usize>) -> Self {
        Collapser {
            arr_th,
            obj_th,
            collapses: 0,
        }
    }

    fn walk(&mut self, val: Value) -> Value {
        match val {
            Value::Array(arr) => self.walk_array(arr),
            Value::Object(obj) => self.walk_object(obj),
            other => other,
        }
    }

    fn walk_array(&mut self, arr: Vec<Value>) -> Value {
        let mut walked = Vec::with_capacity(arr.len());
        for val in arr {
            walked.push(self.walk(val));
        }

        if walked.len() >= self.arr_th {
            let mut result = Vec::new();
            // Head 3
            result.extend(walked.iter().take(3).cloned());
            
            // Marker
            let omitted = walked.len() - 5;
            let omitted_original = &walked[3..walked.len()-2];
            let marker_text = format!(
                "[... {} items omitted; types: {} ...]",
                omitted,
                type_counts(omitted_original)
            );
            result.push(Value::String(marker_text));

            // Tail 2
            result.extend(walked.iter().skip(walked.len() - 2).cloned());
            
            self.collapses += 1;
            Value::Array(result)
        } else {
            Value::Array(walked)
        }
    }

    fn walk_object(&mut self, obj: Map<String, Value>) -> Value {
        let mut walked = Map::new();
        for (k, v) in obj {
            walked.insert(k, self.walk(v));
        }

        if let Some(obj_th) = self.obj_th {
            if walked.len() >= obj_th {
                let keys: Vec<String> = walked.keys().cloned().collect();
                let head_keys = &keys[..5];
                let tail_keys = &keys[keys.len()-3..];
                let omitted = walked.len() - 8;

                // Make sure we select an omitted key that doesn't collide
                let mut kept = head_keys.iter().collect::<HashSet<_>>();
                for k in tail_keys {
                    kept.insert(k);
                }

                let mut marker_key = OMITTED_KEY.to_string();
                if kept.contains(&marker_key) {
                    let mut i = 2;
                    while kept.contains(&format!("{}_{}", OMITTED_KEY, i)) {
                        i += 1;
                    }
                    marker_key = format!("{}_{}", OMITTED_KEY, i);
                }

                let mut result = Map::new();
                for k in head_keys {
                    if let Some(v) = walked.get(k) {
                        result.insert(k.clone(), v.clone());
                    }
                }
                result.insert(
                    marker_key,
                    Value::String(format!("[... {} keys omitted ...]", omitted))
                );
                for k in tail_keys {
                    if let Some(v) = walked.get(k) {
                        result.insert(k.clone(), v.clone());
                    }
                }

                self.collapses += 1;
                return Value::Object(result);
            }
        }
        Value::Object(walked)
    }
}

// Simple std::collections::HashSet proxy for fast contains checking
use std::collections::HashSet;

pub fn compress_json_collapse(text: &str, level: &str) -> Result<EngineResult, String> {
    if !["off", "lite", "standard", "aggressive"].contains(&level) {
        return Err(format!("level must be off, lite, standard, or aggressive, got {:?}", level));
    }
    if level == "off" || text.is_empty() {
        return Ok(identity_result(text));
    }

    let (arr_th, obj_th) = level_config(level);
    let mut collapser = Collapser::new(arr_th, obj_th);

    // Only attempt to parse if it looks like a JSON block (array or object)
    let trimmed = text.trim();
    if !((trimmed.starts_with('{') && trimmed.ends_with('}')) || (trimmed.starts_with('[') && trimmed.ends_with(']'))) {
        return Ok(identity_result(text));
    }

    let parsed: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Ok(identity_result(text)),
    };

    let new_struct = collapser.walk(parsed);
    if collapser.collapses == 0 {
        return Ok(identity_result(text));
    }

    // Default compact serialization
    let out_text = serde_json::to_string(&new_struct).unwrap_or_default();
    if out_text.chars().count() >= text.chars().count() {
        return Ok(identity_result(text));
    }

    let note = format!("{} structures collapsed", collapser.collapses);
    Ok(make_result(text, &out_text, note))
}
