use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, warn};

/// All event types emitted by `claude --output-format stream-json`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeEvent {
    System {
        subtype: String,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
    Assistant {
        message: AssistantMessage,
    },
    User {
        message: UserMessage,
    },
    Result {
        subtype: String,
        #[serde(default)]
        cost_usd: Option<f64>,
        duration_ms: u64,
        num_turns: u32,
        usage: Option<TokenUsage>,
        is_error: bool,
        #[serde(default)]
        result: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub id: Option<String>,
    pub model: Option<String>,
    pub content: Vec<ContentBlock>,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        is_error: bool,
        content: Vec<ToolResultContent>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResultContent {
    Text { text: String },
    Image { source: serde_json::Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
    pub cache_creation_input_tokens: Option<u32>,
}

impl TokenUsage {
    pub fn context_used(&self) -> u32 {
        self.input_tokens.unwrap_or(0)
            + self.cache_read_input_tokens.unwrap_or(0)
            + self.cache_creation_input_tokens.unwrap_or(0)
    }
}

/// Derived summary of a file modification inferred from tool use
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub tool: String,
    pub path: String,
    pub kind: FileChangeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Create,
    Edit,
    Delete,
}

impl FileChange {
    pub fn from_tool_use(name: &str, input: &serde_json::Value) -> Option<Self> {
        let path = input.get("file_path")
            .or_else(|| input.get("path"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)?;

        let kind = match name {
            "Write" => FileChangeKind::Create,
            "Edit" | "MultiEdit" => FileChangeKind::Edit,
            _ => return None,
        };

        Some(FileChange {
            tool: name.to_string(),
            path,
            kind,
        })
    }
}

/// Accumulated session state derived from the event stream
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SessionAccumulator {
    pub session_id: String,
    pub total_cost_usd: f64,
    pub turn_count: u32,
    pub file_changes: Vec<FileChange>,
    pub last_usage: Option<TokenUsage>,
    pub is_complete: bool,
    pub is_error: bool,
    pub duration: Option<Duration>,
}

/// Parses JSONL stream from `claude --output-format stream-json`
pub struct ClaudeStreamParser {
    tx: broadcast::Sender<ClaudeEvent>,
    pub accumulator: SessionAccumulator,
}

impl ClaudeStreamParser {
    pub fn new(session_id: String) -> (Self, broadcast::Receiver<ClaudeEvent>) {
        let (tx, rx) = broadcast::channel(256);
        let parser = Self {
            tx,
            accumulator: SessionAccumulator {
                session_id,
                ..Default::default()
            },
        };
        (parser, rx)
    }

    /// Process a single JSONL line. Silently skips blank lines.
    pub fn process_line(&mut self, line: &str) -> Result<Option<ClaudeEvent>> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let event: ClaudeEvent = match serde_json::from_str(trimmed) {
            Ok(e) => e,
            Err(err) => {
                warn!("Failed to parse claude stream-json line: {} | line: {}", err, trimmed);
                return Ok(None);
            }
        };

        debug!("Claude event: {:?}", event);

        // Update accumulator
        match &event {
            ClaudeEvent::Assistant { message } => {
                self.accumulator.turn_count += 1;
                if let Some(usage) = &message.usage {
                    self.accumulator.last_usage = Some(usage.clone());
                }
                for block in &message.content {
                    if let ContentBlock::ToolUse { name, input, .. } = block {
                        if let Some(change) = FileChange::from_tool_use(name, input) {
                            self.accumulator.file_changes.push(change);
                        }
                    }
                }
            }
            ClaudeEvent::Result { cost_usd, duration_ms, is_error, .. } => {
                if let Some(cost) = cost_usd {
                    self.accumulator.total_cost_usd += cost;
                }
                self.accumulator.duration = Some(Duration::from_millis(*duration_ms));
                self.accumulator.is_complete = true;
                self.accumulator.is_error = *is_error;
            }
            _ => {}
        }

        // Broadcast to subscribers (ignore if no receivers)
        let _ = self.tx.send(event.clone());

        Ok(Some(event))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ClaudeEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_parser() -> ClaudeStreamParser {
        let (p, _) = ClaudeStreamParser::new("test-session".to_string());
        p
    }

    #[test]
    fn parse_text_block() {
        let mut parser = make_parser();
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello!"}],"usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let event = parser.process_line(line).unwrap().unwrap();
        assert!(matches!(event, ClaudeEvent::Assistant { .. }));
        assert_eq!(parser.accumulator.turn_count, 1);
    }

    #[test]
    fn parse_tool_use_extracts_file_change() {
        let mut parser = make_parser();
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Write","input":{"file_path":"src/main.rs","content":"fn main(){}"}}]}}"#;
        parser.process_line(line).unwrap();
        assert_eq!(parser.accumulator.file_changes.len(), 1);
        assert_eq!(parser.accumulator.file_changes[0].path, "src/main.rs");
        assert_eq!(parser.accumulator.file_changes[0].kind, FileChangeKind::Create);
    }

    #[test]
    fn parse_result_accumulates_cost() {
        let mut parser = make_parser();
        let line = r#"{"type":"result","subtype":"success","cost_usd":0.42,"duration_ms":5000,"num_turns":3,"is_error":false}"#;
        parser.process_line(line).unwrap();
        assert!((parser.accumulator.total_cost_usd - 0.42).abs() < 1e-9);
        assert!(parser.accumulator.is_complete);
        assert!(!parser.accumulator.is_error);
    }

    #[test]
    fn blank_lines_are_ignored() {
        let mut parser = make_parser();
        assert!(parser.process_line("").unwrap().is_none());
        assert!(parser.process_line("   ").unwrap().is_none());
    }

    #[test]
    fn malformed_json_is_skipped() {
        let mut parser = make_parser();
        let result = parser.process_line("not json {{{");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn multi_turn_cost_accumulates() {
        let mut parser = make_parser();
        let r1 = r#"{"type":"result","subtype":"success","cost_usd":0.10,"duration_ms":1000,"num_turns":1,"is_error":false}"#;
        let r2 = r#"{"type":"result","subtype":"success","cost_usd":0.25,"duration_ms":2000,"num_turns":2,"is_error":false}"#;
        parser.process_line(r1).unwrap();
        parser.process_line(r2).unwrap();
        assert!((parser.accumulator.total_cost_usd - 0.35).abs() < 1e-9);
    }
}
