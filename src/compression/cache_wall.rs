use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockKind {
    System,
    Tools,
    Message,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallMarker {
    pub kind: BlockKind,
    pub msg_idx: Option<usize>,
    pub block_idx: usize,
}

impl WallMarker {
    fn order_key(&self) -> (u8, i32, usize) {
        let kind_val = match self.kind {
            BlockKind::System => 0,
            BlockKind::Tools => 1,
            BlockKind::Message => 2,
        };
        let msg_val = match self.msg_idx {
            Some(idx) => idx as i32,
            None => -1,
        };
        (kind_val, msg_val, self.block_idx)
    }
}

impl PartialOrd for WallMarker {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for WallMarker {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order_key().cmp(&other.order_key())
    }
}

#[derive(Debug, Clone)]
pub struct CacheWall {
    pub marker: Option<WallMarker>,
    pub auto_inserted: bool,
    pub all_markers: Vec<WallMarker>,
}

impl CacheWall {
    pub fn has_marker(&self) -> bool {
        self.marker.is_some()
    }

    pub fn is_protected(
        &self,
        kind: BlockKind,
        msg_idx: Option<usize>,
        block_idx: usize,
    ) -> bool {
        let m = match &self.marker {
            Some(marker) => marker,
            None => return false,
        };

        let kind_val = match kind {
            BlockKind::System => 0,
            BlockKind::Tools => 1,
            BlockKind::Message => 2,
        };
        let m_kind_val = match m.kind {
            BlockKind::System => 0,
            BlockKind::Tools => 1,
            BlockKind::Message => 2,
        };

        if kind_val < m_kind_val {
            return true;
        }
        if kind_val > m_kind_val {
            return false;
        }

        // Same kind — compare within-kind position.
        match kind {
            BlockKind::System | BlockKind::Tools => block_idx <= m.block_idx,
            BlockKind::Message => {
                let msg_val = msg_idx.unwrap_or(0);
                let m_msg_val = m.msg_idx.unwrap_or(0);
                if msg_val < m_msg_val {
                    return true;
                }
                if msg_val > m_msg_val {
                    return false;
                }
                block_idx <= m.block_idx
            }
        }
    }
}

fn find_all_markers(payload: &Value) -> Vec<WallMarker> {
    let mut found = Vec::new();

    if let Some(system) = payload.get("system") {
        if let Some(system_arr) = system.as_array() {
            for (i, block) in system_arr.iter().enumerate() {
                if block.get("cache_control").is_some() {
                    found.push(WallMarker {
                        kind: BlockKind::System,
                        msg_idx: None,
                        block_idx: i,
                    });
                }
            }
        }
    }

    if let Some(tools) = payload.get("tools") {
        if let Some(tools_arr) = tools.as_array() {
            for (i, tool) in tools_arr.iter().enumerate() {
                if tool.get("cache_control").is_some() {
                    found.push(WallMarker {
                        kind: BlockKind::Tools,
                        msg_idx: None,
                        block_idx: i,
                    });
                }
            }
        }
    }

    if let Some(messages) = payload.get("messages") {
        if let Some(messages_arr) = messages.as_array() {
            for (mi, message) in messages_arr.iter().enumerate() {
                if let Some(content) = message.get("content") {
                    if let Some(content_arr) = content.as_array() {
                        for (bi, block) in content_arr.iter().enumerate() {
                            if block.get("cache_control").is_some() {
                                found.push(WallMarker {
                                    kind: BlockKind::Message,
                                    msg_idx: Some(mi),
                                    block_idx: bi,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    found
}

fn last_marker(markers: &[WallMarker]) -> Option<WallMarker> {
    markers.iter().max().cloned()
}

fn auto_insert_breakpoint(payload: &mut Value) -> Option<WallMarker> {
    let breakpoint = serde_json::json!({"type": "ephemeral"});

    if let Some(tools) = payload.get_mut("tools") {
        if let Some(tools_arr) = tools.as_array_mut() {
            if !tools_arr.is_empty() {
                let last_idx = tools_arr.len() - 1;
                if let Some(last) = tools_arr.get_mut(last_idx) {
                    if let Some(last_obj) = last.as_object_mut() {
                        last_obj.insert("cache_control".to_string(), breakpoint);
                        return Some(WallMarker {
                            kind: BlockKind::Tools,
                            msg_idx: None,
                            block_idx: last_idx,
                        });
                    }
                }
            }
        }
    }

    if let Some(system) = payload.get_mut("system") {
        if let Some(system_arr) = system.as_array_mut() {
            if !system_arr.is_empty() {
                let last_idx = system_arr.len() - 1;
                if let Some(last) = system_arr.get_mut(last_idx) {
                    if let Some(last_obj) = last.as_object_mut() {
                        last_obj.insert("cache_control".to_string(), breakpoint);
                        return Some(WallMarker {
                            kind: BlockKind::System,
                            msg_idx: None,
                            block_idx: last_idx,
                        });
                    }
                }
            }
        } else if let Some(system_str) = system.as_str() {
            if !system_str.is_empty() {
                *system = serde_json::json!([
                    {
                        "type": "text",
                        "text": system_str,
                        "cache_control": breakpoint
                    }
                ]);
                return Some(WallMarker {
                    kind: BlockKind::System,
                    msg_idx: None,
                    block_idx: 0,
                });
            }
        }
    }

    None
}

pub fn compute_wall(payload: &mut Value, auto_insert: bool) -> CacheWall {
    let markers = find_all_markers(payload);
    let last = last_marker(&markers);

    if last.is_some() || !auto_insert {
        return CacheWall {
            marker: last,
            auto_inserted: false,
            all_markers: markers,
        };
    }

    if let Some(inserted) = auto_insert_breakpoint(payload) {
        CacheWall {
            marker: Some(inserted.clone()),
            auto_inserted: true,
            all_markers: vec![inserted],
        }
    } else {
        CacheWall {
            marker: None,
            auto_inserted: false,
            all_markers: Vec::new(),
        }
    }
}

pub fn assert_prefix_unchanged(
    original: &[u8],
    outgoing: &[u8],
    wall: &CacheWall,
    prefix_len: Option<usize>,
) -> Result<(), String> {
    if !wall.has_marker() {
        return Ok(());
    }
    let limit = prefix_len.unwrap_or_else(|| std::cmp::min(original.len(), outgoing.len()));
    if original[..limit] != outgoing[..limit] {
        for i in 0..limit {
            if original[i] != outgoing[i] {
                let start_idx = i.saturating_sub(16);
                let end_idx = std::cmp::min(limit, i + 16);
                return Err(format!(
                    "Cache prefix mutated at byte {}: original={:?}, outgoing={:?}",
                    i,
                    String::from_utf8_lossy(&original[start_idx..end_idx]),
                    String::from_utf8_lossy(&outgoing[start_idx..end_idx])
                ));
            }
        }
    }
    Ok(())
}
