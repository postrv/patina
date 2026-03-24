//! Language server discovery and auto-detection.
//!
//! Detects installed language servers by probing well-known binary names
//! on the system PATH. Maps file extensions to languages and languages
//! to their preferred servers.

use std::process::Command;

/// Information about a discovered language server.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    /// Human-readable server name.
    pub name: String,
    /// Language this server handles.
    pub language: String,
    /// Command and arguments to start the server.
    pub command: Vec<String>,
}

/// Known language servers and their detection commands.
const KNOWN_SERVERS: &[(&str, &str, &[&str])] = &[
    ("rust-analyzer", "rust", &["rust-analyzer"]),
    (
        "typescript-language-server",
        "typescript",
        &["typescript-language-server", "--stdio"],
    ),
    ("pyright", "python", &["pyright-langserver", "--stdio"]),
    ("gopls", "go", &["gopls", "serve"]),
    ("clangd", "c", &["clangd"]),
    ("lua-language-server", "lua", &["lua-language-server"]),
];

/// Maps a file extension to a language name.
///
/// Returns `None` for unrecognized extensions.
#[must_use]
pub fn language_from_extension(ext: &str) -> Option<String> {
    let lang = match ext.to_lowercase().as_str() {
        "rs" => "rust",
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => "typescript",
        "py" | "pyi" => "python",
        "go" => "go",
        "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" => "c",
        "lua" => "lua",
        _ => return None,
    };
    Some(lang.to_string())
}

/// Discovers a language server for the given language.
///
/// Checks if the server binary exists on the system PATH.
/// Returns `None` if no server is found.
#[must_use]
pub fn discover_server(language: &str) -> Option<ServerInfo> {
    for &(name, lang, cmd) in KNOWN_SERVERS {
        if lang == language && is_binary_available(cmd[0]) {
            return Some(ServerInfo {
                name: name.to_string(),
                language: lang.to_string(),
                command: cmd.iter().map(|s| (*s).to_string()).collect(),
            });
        }
    }
    None
}

/// Checks if a binary is available on the system PATH.
fn is_binary_available(binary: &str) -> bool {
    Command::new("which")
        .arg(binary)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Returns all known server configurations.
///
/// Useful for displaying available servers in help text.
#[must_use]
pub fn known_servers() -> Vec<ServerInfo> {
    KNOWN_SERVERS
        .iter()
        .map(|&(name, lang, cmd)| ServerInfo {
            name: name.to_string(),
            language: lang.to_string(),
            command: cmd.iter().map(|s| (*s).to_string()).collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_extension_rust() {
        assert_eq!(language_from_extension("rs"), Some("rust".to_string()));
    }

    #[test]
    fn test_language_from_extension_typescript_variants() {
        assert_eq!(
            language_from_extension("ts"),
            Some("typescript".to_string())
        );
        assert_eq!(
            language_from_extension("tsx"),
            Some("typescript".to_string())
        );
        assert_eq!(
            language_from_extension("js"),
            Some("typescript".to_string())
        );
        assert_eq!(
            language_from_extension("jsx"),
            Some("typescript".to_string())
        );
    }

    #[test]
    fn test_language_from_extension_python() {
        assert_eq!(language_from_extension("py"), Some("python".to_string()));
        assert_eq!(language_from_extension("pyi"), Some("python".to_string()));
    }

    #[test]
    fn test_language_from_extension_go() {
        assert_eq!(language_from_extension("go"), Some("go".to_string()));
    }

    #[test]
    fn test_language_from_extension_c() {
        assert_eq!(language_from_extension("c"), Some("c".to_string()));
        assert_eq!(language_from_extension("cpp"), Some("c".to_string()));
        assert_eq!(language_from_extension("h"), Some("c".to_string()));
    }

    #[test]
    fn test_language_from_extension_unknown() {
        assert_eq!(language_from_extension("xyz"), None);
        assert_eq!(language_from_extension(""), None);
    }

    #[test]
    fn test_language_from_extension_case_insensitive() {
        assert_eq!(language_from_extension("RS"), Some("rust".to_string()));
        assert_eq!(language_from_extension("Py"), Some("python".to_string()));
    }

    #[test]
    fn test_known_servers_not_empty() {
        let servers = known_servers();
        assert!(!servers.is_empty());
        assert!(servers.iter().any(|s| s.name == "rust-analyzer"));
    }

    #[test]
    fn test_known_servers_have_commands() {
        for server in known_servers() {
            assert!(!server.command.is_empty(), "{} has no command", server.name);
            assert!(
                !server.language.is_empty(),
                "{} has no language",
                server.name
            );
        }
    }

    #[test]
    fn test_discover_server_unknown_language() {
        assert!(discover_server("brainfuck").is_none());
    }
}
