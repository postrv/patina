//! Tool definitions for the Anthropic API.
//!
//! This module provides tool schemas that are sent to the Anthropic API
//! to enable Claude to make tool_use calls instead of outputting XML-like text.
//!
//! # Overview
//!
//! The Anthropic API requires tools to be defined in the request payload
//! with a specific JSON schema format. Without these definitions, Claude
//! will improvise with text-based tool invocations like `<bash>command</bash>`.
//!
//! # Example
//!
//! ```rust
//! use patina::api::tools::{ToolDefinition, default_tools};
//!
//! let tools = default_tools();
//! assert!(tools.iter().any(|t| t.name == "bash"));
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A tool definition for the Anthropic API.
///
/// This struct represents the schema that Claude uses to understand
/// how to invoke a tool. The API returns `tool_use` content blocks
/// when it wants to call a tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    /// The unique name of the tool (e.g., "bash", "read_file").
    pub name: String,

    /// Human-readable description of what the tool does.
    /// Claude uses this to decide when to use the tool.
    pub description: String,

    /// JSON Schema defining the input parameters.
    /// Must be a valid JSON Schema object with "type": "object".
    pub input_schema: Value,
}

impl ToolDefinition {
    /// Creates a new tool definition.
    ///
    /// # Arguments
    ///
    /// * `name` - The tool's unique identifier
    /// * `description` - What the tool does (helps Claude decide when to use it)
    /// * `input_schema` - JSON Schema for the tool's parameters
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// Tool choice configuration for the API request.
///
/// Controls how Claude selects tools during a conversation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    /// Claude decides whether to use tools based on the conversation.
    #[default]
    Auto,
    /// Claude must use a tool (any tool).
    Any,
    /// Claude must use a specific tool.
    Tool {
        /// The name of the required tool.
        name: String,
    },
}

/// Returns the default set of tools for Patina.
///
/// Includes: bash, read_file, write_file, edit, list_files, glob, grep, web_fetch, web_search, analyze_image
#[must_use]
pub fn default_tools() -> Vec<ToolDefinition> {
    vec![
        bash_tool(),
        read_file_tool(),
        write_file_tool(),
        edit_tool(),
        list_files_tool(),
        glob_tool(),
        grep_tool(),
        web_fetch_tool(),
        web_search_tool(),
        vision_tool(),
    ]
}

/// Creates the bash tool definition.
///
/// Executes shell commands in the working directory.
#[must_use]
pub fn bash_tool() -> ToolDefinition {
    ToolDefinition::new(
        "bash",
        "Execute a bash command in the working directory. Use for running shell commands, \
         git operations, build tools, and system utilities. Commands run with a timeout \
         and certain dangerous operations are blocked for security.",
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                }
            },
            "required": ["command"]
        }),
    )
}

/// Creates the read_file tool definition.
///
/// Reads the contents of a file.
#[must_use]
pub fn read_file_tool() -> ToolDefinition {
    ToolDefinition::new(
        "read_file",
        "Read the contents of a file. The path must be relative to the working directory. \
         Returns the full file content as text. Binary files may not read correctly.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The relative path to the file to read"
                }
            },
            "required": ["path"]
        }),
    )
}

/// Creates the write_file tool definition.
///
/// Writes content to a file, creating it if it doesn't exist.
#[must_use]
pub fn write_file_tool() -> ToolDefinition {
    ToolDefinition::new(
        "write_file",
        "Write content to a file. Creates the file if it doesn't exist, or overwrites \
         if it does. The path must be relative to the working directory. Parent \
         directories are created automatically.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The relative path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["path", "content"]
        }),
    )
}

/// Creates the edit tool definition.
///
/// Performs a search-and-replace edit on a file.
#[must_use]
pub fn edit_tool() -> ToolDefinition {
    ToolDefinition::new(
        "edit",
        "Edit a file by replacing a specific string with another. The old_string must \
         match exactly once in the file (unique match required). Use this for precise \
         modifications rather than rewriting entire files.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The relative path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace (must be unique in file)"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace old_string with"
                }
            },
            "required": ["path", "old_string", "new_string"]
        }),
    )
}

/// Creates the list_files tool definition.
///
/// Lists files and directories in a given path.
#[must_use]
pub fn list_files_tool() -> ToolDefinition {
    ToolDefinition::new(
        "list_files",
        "List files and directories at a given path. Returns entries prefixed with \
         'd ' for directories and '- ' for files. The path must be relative to the \
         working directory.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The relative path to the directory to list (default: current directory)"
                }
            },
            "required": []
        }),
    )
}

/// Creates the glob tool definition.
///
/// Finds files matching a glob pattern.
#[must_use]
pub fn glob_tool() -> ToolDefinition {
    ToolDefinition::new(
        "glob",
        "Find files matching a glob pattern. Supports patterns like '**/*.rs' for \
         recursive search, '*.txt' for current directory, and 'src/**/*.ts' for \
         specific subdirectories. Returns matching file paths.",
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against (e.g., '**/*.rs')"
                },
                "respect_gitignore": {
                    "type": "boolean",
                    "description": "Whether to respect .gitignore rules (default: false)"
                }
            },
            "required": ["pattern"]
        }),
    )
}

/// Creates the grep tool definition.
///
/// Searches file contents for a regex pattern.
#[must_use]
pub fn grep_tool() -> ToolDefinition {
    ToolDefinition::new(
        "grep",
        "Search file contents for a regular expression pattern. Returns matching lines \
         with file paths and line numbers. Useful for finding code references, function \
         definitions, and text patterns across the codebase.",
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Optional glob pattern to filter which files to search (e.g., '*.rs')"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Whether to perform case-insensitive search (default: false)"
                }
            },
            "required": ["pattern"]
        }),
    )
}

/// Creates the web_fetch tool definition.
///
/// Fetches content from a URL and converts HTML to markdown.
#[must_use]
pub fn web_fetch_tool() -> ToolDefinition {
    ToolDefinition::new(
        "web_fetch",
        "Fetch content from a URL. HTML content is automatically converted to markdown \
         for better readability. The URL must be http or https - file:// and localhost \
         URLs are blocked for security. Has a 30 second timeout and 1MB content limit.",
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from (must be http or https)"
                }
            },
            "required": ["url"]
        }),
    )
}

/// Creates the web_search tool definition.
///
/// Searches the web using DuckDuckGo and returns formatted results.
#[must_use]
pub fn web_search_tool() -> ToolDefinition {
    ToolDefinition::new(
        "web_search",
        "Search the web for information. Returns search results with titles, URLs, and \
         snippets formatted as markdown. Use for finding documentation, answering questions, \
         or discovering resources. Has a 30 second timeout and returns up to 10 results.",
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query to find results for"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 10)",
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "required": ["query"]
        }),
    )
}

/// Creates the analyze_image (vision) tool definition.
///
/// Analyzes images using Claude's vision capabilities.
#[must_use]
pub fn vision_tool() -> ToolDefinition {
    ToolDefinition::new(
        "analyze_image",
        "Analyze an image using Claude's vision capabilities. Load an image from a file path \
         and optionally provide a prompt to guide the analysis. Supported formats: PNG, JPEG, \
         GIF, WebP. Maximum file size: 20MB.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The relative path to the image file to analyze"
                },
                "prompt": {
                    "type": "string",
                    "description": "Optional prompt to guide the image analysis (e.g., 'What objects are in this image?')"
                }
            },
            "required": ["path"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_definition_new() {
        let tool = ToolDefinition::new(
            "test_tool",
            "A test tool",
            json!({"type": "object", "properties": {}}),
        );

        assert_eq!(tool.name, "test_tool");
        assert_eq!(tool.description, "A test tool");
        assert_eq!(tool.input_schema["type"], "object");
    }

    #[test]
    fn test_tool_definition_serialization() {
        let tool = bash_tool();
        let json = serde_json::to_string(&tool).expect("serialization should succeed");

        assert!(json.contains("\"name\":\"bash\""));
        assert!(json.contains("\"description\":"));
        assert!(json.contains("\"input_schema\":"));
    }

    #[test]
    fn test_tool_definition_deserialization() {
        let json = r#"{
            "name": "bash",
            "description": "Run commands",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"]
            }
        }"#;

        let tool: ToolDefinition =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(tool.name, "bash");
        assert_eq!(tool.input_schema["properties"]["command"]["type"], "string");
    }

    #[test]
    fn test_default_tools_contains_all_tools() {
        let tools = default_tools();

        assert_eq!(tools.len(), 10, "should have 10 default tools");

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"bash"), "should contain bash");
        assert!(names.contains(&"read_file"), "should contain read_file");
        assert!(names.contains(&"write_file"), "should contain write_file");
        assert!(names.contains(&"edit"), "should contain edit");
        assert!(names.contains(&"list_files"), "should contain list_files");
        assert!(names.contains(&"glob"), "should contain glob");
        assert!(names.contains(&"grep"), "should contain grep");
        assert!(names.contains(&"web_fetch"), "should contain web_fetch");
        assert!(names.contains(&"web_search"), "should contain web_search");
        assert!(
            names.contains(&"analyze_image"),
            "should contain analyze_image"
        );
    }

    #[test]
    fn test_bash_tool_schema() {
        let tool = bash_tool();

        assert_eq!(tool.name, "bash");
        assert!(tool.description.contains("bash"));

        let schema = &tool.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["command"].is_object());
        assert_eq!(schema["required"], json!(["command"]));
    }

    #[test]
    fn test_read_file_tool_schema() {
        let tool = read_file_tool();

        assert_eq!(tool.name, "read_file");

        let schema = &tool.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
        assert_eq!(schema["required"], json!(["path"]));
    }

    #[test]
    fn test_write_file_tool_schema() {
        let tool = write_file_tool();

        assert_eq!(tool.name, "write_file");

        let schema = &tool.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["content"].is_object());
        assert_eq!(schema["required"], json!(["path", "content"]));
    }

    #[test]
    fn test_edit_tool_schema() {
        let tool = edit_tool();

        assert_eq!(tool.name, "edit");

        let schema = &tool.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["old_string"].is_object());
        assert!(schema["properties"]["new_string"].is_object());
        assert_eq!(
            schema["required"],
            json!(["path", "old_string", "new_string"])
        );
    }

    #[test]
    fn test_list_files_tool_schema() {
        let tool = list_files_tool();

        assert_eq!(tool.name, "list_files");

        let schema = &tool.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
        // path is optional for list_files
        assert_eq!(schema["required"], json!([]));
    }

    #[test]
    fn test_glob_tool_schema() {
        let tool = glob_tool();

        assert_eq!(tool.name, "glob");

        let schema = &tool.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["pattern"].is_object());
        assert!(schema["properties"]["respect_gitignore"].is_object());
        assert_eq!(schema["required"], json!(["pattern"]));
    }

    #[test]
    fn test_grep_tool_schema() {
        let tool = grep_tool();

        assert_eq!(tool.name, "grep");

        let schema = &tool.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["pattern"].is_object());
        assert!(schema["properties"]["file_pattern"].is_object());
        assert!(schema["properties"]["case_insensitive"].is_object());
        assert_eq!(schema["required"], json!(["pattern"]));
    }

    #[test]
    fn test_web_fetch_tool_schema() {
        let tool = web_fetch_tool();

        assert_eq!(tool.name, "web_fetch");
        assert!(tool.description.contains("URL"));

        let schema = &tool.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["url"].is_object());
        assert_eq!(schema["required"], json!(["url"]));
    }

    #[test]
    fn test_tool_choice_auto_serialization() {
        let choice = ToolChoice::Auto;
        let json = serde_json::to_string(&choice).expect("serialization should succeed");
        assert_eq!(json, r#"{"type":"auto"}"#);
    }

    #[test]
    fn test_tool_choice_any_serialization() {
        let choice = ToolChoice::Any;
        let json = serde_json::to_string(&choice).expect("serialization should succeed");
        assert_eq!(json, r#"{"type":"any"}"#);
    }

    #[test]
    fn test_tool_choice_tool_serialization() {
        let choice = ToolChoice::Tool {
            name: "bash".to_string(),
        };
        let json = serde_json::to_string(&choice).expect("serialization should succeed");
        assert!(json.contains(r#""type":"tool""#));
        assert!(json.contains(r#""name":"bash""#));
    }

    #[test]
    fn test_tool_choice_default_is_auto() {
        let choice = ToolChoice::default();
        assert_eq!(choice, ToolChoice::Auto);
    }

    #[test]
    fn test_all_tools_have_object_type_schema() {
        for tool in default_tools() {
            assert_eq!(
                tool.input_schema["type"], "object",
                "tool {} should have object type schema",
                tool.name
            );
        }
    }

    #[test]
    fn test_all_tools_have_properties() {
        for tool in default_tools() {
            assert!(
                tool.input_schema["properties"].is_object(),
                "tool {} should have properties object",
                tool.name
            );
        }
    }

    #[test]
    fn test_all_tools_have_required_array() {
        for tool in default_tools() {
            assert!(
                tool.input_schema["required"].is_array(),
                "tool {} should have required array",
                tool.name
            );
        }
    }

    #[test]
    fn test_tools_are_clone_and_debug() {
        let tool = bash_tool();
        let cloned = tool.clone();
        assert_eq!(tool, cloned);

        let debug = format!("{:?}", tool);
        assert!(debug.contains("bash"));
    }

    #[test]
    fn test_tool_names_match_executor() {
        // These names must match what ToolExecutor.execute() expects
        let expected_names = [
            "bash",
            "read_file",
            "write_file",
            "edit",
            "list_files",
            "glob",
            "grep",
            "web_fetch",
        ];
        let tools = default_tools();

        for expected in expected_names {
            assert!(
                tools.iter().any(|t| t.name == expected),
                "missing tool definition for executor tool: {}",
                expected
            );
        }
    }
}
