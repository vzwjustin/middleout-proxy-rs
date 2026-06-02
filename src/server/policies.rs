use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompressionPolicy {
    #[serde(default = "default_true")]
    pub input_compression: bool,
    #[serde(default = "default_true")]
    pub jl_dedupe: bool,
    #[serde(default = "default_false")]
    pub caveman_enabled: bool,
    #[serde(default = "default_caveman_level")]
    pub caveman_level: String,
    #[serde(default = "default_false")]
    pub rtk_enabled: bool,
    #[serde(default = "default_rtk_level")]
    pub rtk_level: String,
    #[serde(default = "default_false")]
    pub output_compression: bool,
    pub max_text_chars: Option<usize>,
}

fn default_true() -> bool { true }
fn default_false() -> bool { false }
fn default_caveman_level() -> String { "standard".to_string() }
fn default_rtk_level() -> String { "minimal".to_string() }

impl Default for CompressionPolicy {
    fn default() -> Self {
        CompressionPolicy {
            input_compression: true,
            jl_dedupe: true,
            caveman_enabled: false,
            caveman_level: "standard".to_string(),
            rtk_enabled: false,
            rtk_level: "minimal".to_string(),
            output_compression: false,
            max_text_chars: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyMatch {
    #[serde(default = "default_star")]
    pub model_glob: String,
    #[serde(default = "default_star")]
    pub endpoint: String,
    #[serde(default)]
    pub policy: CompressionPolicy,
}

fn default_star() -> String { "*".to_string() }

pub struct PolicyRouter {
    pub rules: Vec<PolicyMatch>,
    pub default: CompressionPolicy,
}

fn glob_to_regex(glob: &str) -> String {
    let mut regex = String::new();
    regex.push('^');
    for c in glob.chars() {
        match c {
            '*' => regex.push_str(".*"),
            '?' => regex.push_str("."),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '\\' | '|' => {
                regex.push('\\');
                regex.push(c);
            }
            _ => regex.push(c),
        }
    }
    regex.push('$');
    regex
}

impl PolicyRouter {
    pub fn new(rules: Vec<PolicyMatch>, default: Option<CompressionPolicy>) -> Self {
        PolicyRouter {
            rules,
            default: default.unwrap_or_default(),
        }
    }

    pub fn resolve(&self, model: Option<&str>, endpoint: &str) -> CompressionPolicy {
        for rule in &self.rules {
            if rule.endpoint != "*" && rule.endpoint != endpoint {
                continue;
            }

            let matched = match model {
                Some(m) => {
                    let re_str = glob_to_regex(&rule.model_glob);
                    if let Ok(re) = regex::Regex::new(&re_str) {
                        re.is_match(m)
                    } else {
                        false
                    }
                }
                None => rule.model_glob == "*",
            };

            if matched {
                return rule.policy.clone();
            }
        }
        self.default.clone()
    }

    pub fn from_env() -> Self {
        let raw = std::env::var("MIDDLEOUT_POLICIES").unwrap_or_default();
        if raw.trim().is_empty() {
            return PolicyRouter::new(Vec::new(), None);
        }
        Self::from_json(&raw).unwrap_or_else(|_| PolicyRouter::new(Vec::new(), None))
    }

    pub fn from_json(raw: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct RawRouterConfig {
            default: Option<CompressionPolicy>,
            rules: Option<Vec<PolicyMatch>>,
        }

        let config: RawRouterConfig = serde_json::from_str(raw)
            .map_err(|e| format!("Failed to parse policies JSON: {:?}", e))?;

        Ok(PolicyRouter {
            rules: config.rules.unwrap_or_default(),
            default: config.default.unwrap_or_default(),
        })
    }
}
