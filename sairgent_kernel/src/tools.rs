use crate::manifest::CapabilityGrant;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltInToolDefinition {
    pub id: &'static str,
    pub slug: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub tool_kind: &'static str,
    pub provider_slug: &'static str,
    pub required_capability: CapabilityGrant,
    pub assignable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolBindingRecord {
    pub agent_id: String,
    pub tool_slug: String,
    pub name: String,
    pub summary: String,
    pub tool_kind: String,
    pub provider_slug: String,
    pub required_capability: String,
    pub binding_status: String,
}

pub const BUILT_IN_TOOLS: &[BuiltInToolDefinition] = &[
    BuiltInToolDefinition {
        id: "web-search-tavily",
        slug: "web_search_tavily",
        name: "Tavily Web Search",
        summary: "Live web search for current research tasks using Tavily.",
        tool_kind: "web_search",
        provider_slug: "tavily",
        required_capability: CapabilityGrant::WebSearch,
        assignable: true,
    },
    BuiltInToolDefinition {
        id: "web-search-exa",
        slug: "web_search_exa",
        name: "Exa Web Search",
        summary: "Live web search for current research tasks using Exa.",
        tool_kind: "web_search",
        provider_slug: "exa",
        required_capability: CapabilityGrant::WebSearch,
        assignable: true,
    },
];

pub fn built_in_tool_catalog() -> &'static [BuiltInToolDefinition] {
    BUILT_IN_TOOLS
}

pub fn find_built_in_tool(slug: &str) -> Option<&'static BuiltInToolDefinition> {
    BUILT_IN_TOOLS.iter().find(|tool| tool.slug == slug)
}

pub fn tool_kind_slug(slug: &str) -> Option<&'static str> {
    find_built_in_tool(slug).map(|tool| tool.tool_kind)
}

pub fn active_web_search_provider(bindings: &[AgentToolBindingRecord]) -> Option<String> {
    bindings
        .iter()
        .find(|binding| binding.tool_kind == "web_search")
        .map(|binding| binding.provider_slug.clone())
}

pub fn required_capability_slug(capability: &CapabilityGrant) -> String {
    serde_json::to_value(capability)
        .ok()
        .and_then(|value| value.as_str().map(|raw| raw.to_string()))
        .unwrap_or_else(|| format!("{:?}", capability))
}

// ---------------------------------------------------------------------------
// MCP Connector types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Sse,
}

impl McpTransport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Sse => "sse",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "stdio" => Some(Self::Stdio),
            "sse" => Some(Self::Sse),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpConnectorRecord {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub summary: String,
    pub transport: McpTransport,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub url: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub cwd: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentMcpBindingRecord {
    pub agent_id: String,
    pub connector_id: String,
    pub connector_slug: String,
    pub connector_name: String,
    pub transport: String,
    pub binding_status: String,
}

/// Request struct for upserting an MCP connector. Accepted from Tauri commands.
#[derive(Clone, Debug, Deserialize)]
pub struct McpConnectorUpsertRequest {
    pub id: Option<String>,
    pub slug: String,
    pub name: String,
    pub summary: Option<String>,
    pub transport: String,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub url: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub cwd: Option<String>,
    pub enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// CLI Tool types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CliToolRecord {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub summary: Option<String>,
    pub command: String,
    pub args_json: Option<String>,
    pub env_json: Option<String>,
    pub cwd: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Request struct for upserting a CLI tool. Accepted from Tauri commands.
#[derive(Clone, Debug, Deserialize)]
pub struct CliToolUpsertRequest {
    pub id: Option<String>,
    pub slug: String,
    pub name: String,
    pub summary: Option<String>,
    pub command: String,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub cwd: Option<String>,
    pub enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// MCP Connector validation
// ---------------------------------------------------------------------------

pub mod mcp_validation {
    use super::*;
    use std::net::IpAddr;

    const ALLOWED_COMMANDS: &[&str] = &["npx", "uvx", "node", "python3", "python", "deno"];
    const MAX_ARGS: usize = 20;
    const MAX_ARG_LEN: usize = 512;
    const SHELL_METACHARACTERS: &[char] = &[';', '|', '&', '$', '`', '(', ')', '{', '}'];

    const ENV_KEY_BLOCKLIST: &[&str] = &[
        "LD_PRELOAD",
        "PATH",
        "HOME",
        "PYTHONPATH",
        "NODE_OPTIONS",
        "NODE_PATH",
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
    ];

    const ENV_KEY_PREFIX_BLOCKLIST: &[&str] = &["SAIRGENT_", "AGENT_", "DYLD_", "BASH_"];

    const BLOCKED_HEADERS: &[&str] = &[
        "host",
        "transfer-encoding",
        "content-length",
        "cookie",
        "set-cookie",
        "connection",
        "upgrade",
        "proxy-authorization",
    ];

    /// Validate an MCP connector upsert request. Returns Ok(()) if valid,
    /// Err(description) if any security rule is violated.
    pub fn validate_mcp_connector(req: &McpConnectorUpsertRequest) -> Result<(), String> {
        let transport = McpTransport::from_str(&req.transport)
            .ok_or_else(|| format!("Invalid transport '{}': must be 'stdio' or 'sse'", req.transport))?;

        if req.slug.is_empty() {
            return Err("Connector slug must not be empty".to_string());
        }
        if req.name.is_empty() {
            return Err("Connector name must not be empty".to_string());
        }

        match transport {
            McpTransport::Stdio => {
                validate_stdio(req)?;
            }
            McpTransport::Sse => {
                validate_sse(req)?;
            }
        }

        if let Some(ref env) = req.env {
            validate_env(env)?;
        }

        if let Some(ref cwd) = req.cwd {
            validate_cwd(cwd)?;
        }

        Ok(())
    }

    fn validate_stdio(req: &McpConnectorUpsertRequest) -> Result<(), String> {
        let command = req.command.as_deref().ok_or_else(|| {
            "stdio transport requires 'command' field".to_string()
        })?;

        if !ALLOWED_COMMANDS.contains(&command) {
            return Err(format!(
                "Command '{}' not in allowlist: {:?}",
                command, ALLOWED_COMMANDS
            ));
        }

        if let Some(ref args) = req.args {
            if args.len() > MAX_ARGS {
                return Err(format!(
                    "Too many args ({}, max {})",
                    args.len(),
                    MAX_ARGS
                ));
            }
            for (i, arg) in args.iter().enumerate() {
                if arg.len() > MAX_ARG_LEN {
                    return Err(format!(
                        "Arg [{}] exceeds max length ({}, max {})",
                        i,
                        arg.len(),
                        MAX_ARG_LEN
                    ));
                }
                if arg.contains('\0') {
                    return Err(format!("Arg [{}] contains null byte", i));
                }
                for ch in SHELL_METACHARACTERS {
                    if arg.contains(*ch) {
                        return Err(format!(
                            "Arg [{}] contains shell metacharacter '{}'",
                            i, ch
                        ));
                    }
                }
            }
            // Validate first arg (package name) if present
            if let Some(first) = args.first() {
                // Skip flags (start with -)
                if !first.starts_with('-') {
                    validate_package_name(first)?;
                } else if let Some(pkg) = args.iter().find(|a| !a.starts_with('-')) {
                    validate_package_name(pkg)?;
                }
            }
        }

        Ok(())
    }

    fn validate_package_name(name: &str) -> Result<(), String> {
        // Must match ^[@a-zA-Z0-9][-_./a-zA-Z0-9]*$
        if name.is_empty() {
            return Err("Package name must not be empty".to_string());
        }
        let bytes = name.as_bytes();
        let first = bytes[0];
        if !(first == b'@' || first.is_ascii_alphanumeric()) {
            return Err(format!(
                "Package name '{}' must start with @, letter, or digit",
                name
            ));
        }
        for &b in &bytes[1..] {
            if !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'/') {
                return Err(format!(
                    "Package name '{}' contains invalid character '{}'",
                    name, b as char
                ));
            }
        }
        Ok(())
    }

    fn validate_sse(req: &McpConnectorUpsertRequest) -> Result<(), String> {
        let url = req.url.as_deref().ok_or_else(|| {
            "SSE transport requires 'url' field".to_string()
        })?;

        if !url.starts_with("https://") {
            let dev_mode = std::env::var("SAIRGENT_DEV_MODE").unwrap_or_default() == "1";
            if !(dev_mode && url.starts_with("http://")) {
                return Err(format!(
                    "SSE url must use https:// scheme (got '{}')",
                    url
                ));
            }
        }

        // Extract host from URL for private IP check
        // Format: https://host[:port]/path
        let after_scheme = if url.starts_with("https://") {
            &url[8..]
        } else {
            &url[7..] // http:// in dev mode
        };
        let host_port = after_scheme.split('/').next().unwrap_or("");
        let host = if host_port.starts_with('[') {
            // IPv6: [::1]:port
            host_port
                .strip_prefix('[')
                .and_then(|h| h.split(']').next())
                .unwrap_or(host_port)
        } else {
            host_port.split(':').next().unwrap_or(host_port)
        };

        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_ip(&ip) {
                let dev_mode = std::env::var("SAIRGENT_DEV_MODE").unwrap_or_default() == "1";
                if !dev_mode {
                    return Err(format!(
                        "SSE url resolves to private IP '{}' — not allowed in production",
                        ip
                    ));
                }
            }
        } else {
            // hostname: check for localhost
            let lower = host.to_lowercase();
            if lower == "localhost" || lower == "localhost." {
                let dev_mode = std::env::var("SAIRGENT_DEV_MODE").unwrap_or_default() == "1";
                if !dev_mode {
                    return Err(
                        "SSE url points to localhost — not allowed in production".to_string(),
                    );
                }
            }
        }

        // Validate headers
        if let Some(ref headers) = req.headers {
            validate_headers(headers)?;
        }

        Ok(())
    }

    fn is_private_ip(ip: &IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => {
                let octets = v4.octets();
                // 10.x.x.x
                octets[0] == 10
                // 172.16.x.x - 172.31.x.x
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                // 192.168.x.x
                || (octets[0] == 192 && octets[1] == 168)
                // 127.x.x.x
                || octets[0] == 127
            }
            IpAddr::V6(v6) => {
                *v6 == std::net::Ipv6Addr::LOCALHOST
            }
        }
    }

    fn validate_env(env: &HashMap<String, String>) -> Result<(), String> {
        for (key, value) in env {
            // Key must match ^(MCP_|CONNECTOR_)[A-Z0-9_]+$
            if !(key.starts_with("MCP_") || key.starts_with("CONNECTOR_")) {
                return Err(format!(
                    "Env key '{}' must start with MCP_ or CONNECTOR_",
                    key
                ));
            }
            let suffix = if key.starts_with("MCP_") {
                &key[4..]
            } else {
                &key[10..]
            };
            if suffix.is_empty() {
                return Err(format!("Env key '{}' has empty suffix after prefix", key));
            }
            for b in suffix.bytes() {
                if !(b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
                    return Err(format!(
                        "Env key '{}' contains invalid character '{}' — only A-Z, 0-9, _ allowed after prefix",
                        key, b as char
                    ));
                }
            }

            // Blocklist check
            if ENV_KEY_BLOCKLIST.contains(&key.as_str()) {
                return Err(format!("Env key '{}' is blocklisted", key));
            }
            for prefix in ENV_KEY_PREFIX_BLOCKLIST {
                if key.starts_with(prefix) {
                    return Err(format!(
                        "Env key '{}' starts with blocklisted prefix '{}'",
                        key, prefix
                    ));
                }
            }

            // No null bytes in values
            if value.contains('\0') {
                return Err(format!("Env value for key '{}' contains null byte", key));
            }
        }
        Ok(())
    }

    fn validate_cwd(cwd: &str) -> Result<(), String> {
        if cwd.contains("..") {
            return Err("CWD must not contain '..'".to_string());
        }
        if !cwd.starts_with('/') {
            return Err("CWD must be an absolute path".to_string());
        }
        Ok(())
    }

    fn validate_headers(headers: &HashMap<String, String>) -> Result<(), String> {
        for (name, value) in headers {
            let lower = name.to_lowercase();
            if BLOCKED_HEADERS.contains(&lower.as_str()) {
                return Err(format!("Header '{}' is blocklisted", name));
            }
            // Strip \r\n from values is done at storage time; here we just reject them
            if value.contains('\r') || value.contains('\n') {
                return Err(format!(
                    "Header '{}' value contains CR/LF characters",
                    name
                ));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn stdio_req(command: &str, args: Vec<&str>) -> McpConnectorUpsertRequest {
            McpConnectorUpsertRequest {
                id: None,
                slug: "test-connector".to_string(),
                name: "Test Connector".to_string(),
                summary: None,
                transport: "stdio".to_string(),
                command: Some(command.to_string()),
                args: Some(args.into_iter().map(|s| s.to_string()).collect()),
                env: None,
                url: None,
                headers: None,
                cwd: None,
                enabled: None,
            }
        }

        fn sse_req(url: &str) -> McpConnectorUpsertRequest {
            McpConnectorUpsertRequest {
                id: None,
                slug: "test-sse".to_string(),
                name: "Test SSE".to_string(),
                summary: None,
                transport: "sse".to_string(),
                command: None,
                args: None,
                env: None,
                url: Some(url.to_string()),
                headers: None,
                cwd: None,
                enabled: None,
            }
        }

        #[test]
        fn accepts_valid_stdio_npx() {
            let req = stdio_req("npx", vec!["-y", "@modelcontextprotocol/server-filesystem", "/tmp/test"]);
            assert!(validate_mcp_connector(&req).is_ok());
        }

        #[test]
        fn accepts_valid_stdio_uvx() {
            let req = stdio_req("uvx", vec!["mcp-server-git"]);
            assert!(validate_mcp_connector(&req).is_ok());
        }

        #[test]
        fn accepts_valid_stdio_node() {
            let req = stdio_req("node", vec!["server.js"]);
            assert!(validate_mcp_connector(&req).is_ok());
        }

        #[test]
        fn rejects_bash_command() {
            let req = stdio_req("bash", vec!["-c", "echo hello"]);
            let err = validate_mcp_connector(&req).unwrap_err();
            assert!(err.contains("not in allowlist"), "got: {}", err);
        }

        #[test]
        fn rejects_sh_command() {
            let req = stdio_req("sh", vec!["-c", "echo hello"]);
            assert!(validate_mcp_connector(&req).unwrap_err().contains("not in allowlist"));
        }

        #[test]
        fn rejects_curl_command() {
            let req = stdio_req("curl", vec!["https://evil.com"]);
            assert!(validate_mcp_connector(&req).unwrap_err().contains("not in allowlist"));
        }

        #[test]
        fn rejects_args_with_shell_metacharacters() {
            let req = stdio_req("npx", vec!["-y", "pkg; rm -rf /"]);
            let err = validate_mcp_connector(&req).unwrap_err();
            assert!(err.contains("shell metacharacter"), "got: {}", err);
        }

        #[test]
        fn rejects_args_with_pipe() {
            let req = stdio_req("npx", vec!["-y", "pkg | cat /etc/passwd"]);
            assert!(validate_mcp_connector(&req).unwrap_err().contains("shell metacharacter"));
        }

        #[test]
        fn rejects_too_many_args() {
            let args: Vec<&str> = (0..21).map(|_| "arg").collect();
            let req = stdio_req("npx", args);
            assert!(validate_mcp_connector(&req).unwrap_err().contains("Too many args"));
        }

        #[test]
        fn rejects_arg_too_long() {
            let long_arg = "a".repeat(513);
            let req = stdio_req("npx", vec![&long_arg]);
            assert!(validate_mcp_connector(&req).unwrap_err().contains("exceeds max length"));
        }

        #[test]
        fn rejects_null_byte_in_arg() {
            let mut req = stdio_req("npx", vec![]);
            req.args = Some(vec!["hello\0world".to_string()]);
            assert!(validate_mcp_connector(&req).unwrap_err().contains("null byte"));
        }

        #[test]
        fn rejects_ld_preload_env() {
            let mut req = stdio_req("npx", vec!["-y", "some-pkg"]);
            let mut env = HashMap::new();
            env.insert("LD_PRELOAD".to_string(), "/evil.so".to_string());
            req.env = Some(env);
            // LD_PRELOAD does not start with MCP_ or CONNECTOR_, so rejected on prefix
            assert!(validate_mcp_connector(&req).is_err());
        }

        #[test]
        fn rejects_path_env() {
            let mut req = stdio_req("npx", vec!["-y", "some-pkg"]);
            let mut env = HashMap::new();
            env.insert("PATH".to_string(), "/evil".to_string());
            req.env = Some(env);
            assert!(validate_mcp_connector(&req).is_err());
        }

        #[test]
        fn rejects_sairgent_prefix_env() {
            let mut req = stdio_req("npx", vec!["-y", "some-pkg"]);
            let mut env = HashMap::new();
            env.insert("SAIRGENT_SECRET".to_string(), "val".to_string());
            req.env = Some(env);
            assert!(validate_mcp_connector(&req).is_err());
        }

        #[test]
        fn accepts_mcp_prefix_env() {
            let mut req = stdio_req("npx", vec!["-y", "some-pkg"]);
            let mut env = HashMap::new();
            env.insert("MCP_API_KEY".to_string(), "some-key".to_string());
            req.env = Some(env);
            assert!(validate_mcp_connector(&req).is_ok());
        }

        #[test]
        fn accepts_connector_prefix_env() {
            let mut req = stdio_req("npx", vec!["-y", "some-pkg"]);
            let mut env = HashMap::new();
            env.insert("CONNECTOR_TOKEN".to_string(), "tok".to_string());
            req.env = Some(env);
            assert!(validate_mcp_connector(&req).is_ok());
        }

        #[test]
        fn rejects_http_sse_url() {
            let req = sse_req("http://example.com/mcp");
            assert!(validate_mcp_connector(&req).unwrap_err().contains("https://"));
        }

        #[test]
        fn accepts_https_sse_url() {
            let req = sse_req("https://example.com/mcp");
            assert!(validate_mcp_connector(&req).is_ok());
        }

        #[test]
        fn rejects_private_ip_10() {
            let req = sse_req("https://10.0.0.1/mcp");
            assert!(validate_mcp_connector(&req).unwrap_err().contains("private IP"));
        }

        #[test]
        fn rejects_private_ip_172() {
            let req = sse_req("https://172.16.0.1/mcp");
            assert!(validate_mcp_connector(&req).unwrap_err().contains("private IP"));
        }

        #[test]
        fn rejects_private_ip_192() {
            let req = sse_req("https://192.168.1.1/mcp");
            assert!(validate_mcp_connector(&req).unwrap_err().contains("private IP"));
        }

        #[test]
        fn rejects_private_ip_127() {
            let req = sse_req("https://127.0.0.1/mcp");
            assert!(validate_mcp_connector(&req).unwrap_err().contains("private IP"));
        }

        #[test]
        fn rejects_localhost_hostname() {
            let req = sse_req("https://localhost/mcp");
            assert!(validate_mcp_connector(&req).unwrap_err().contains("localhost"));
        }

        #[test]
        fn rejects_blocked_header_host() {
            let mut req = sse_req("https://example.com/mcp");
            let mut headers = HashMap::new();
            headers.insert("Host".to_string(), "evil.com".to_string());
            req.headers = Some(headers);
            assert!(validate_mcp_connector(&req).unwrap_err().contains("blocklisted"));
        }

        #[test]
        fn rejects_blocked_header_cookie() {
            let mut req = sse_req("https://example.com/mcp");
            let mut headers = HashMap::new();
            headers.insert("Cookie".to_string(), "session=abc".to_string());
            req.headers = Some(headers);
            assert!(validate_mcp_connector(&req).unwrap_err().contains("blocklisted"));
        }

        #[test]
        fn rejects_header_with_crlf() {
            let mut req = sse_req("https://example.com/mcp");
            let mut headers = HashMap::new();
            headers.insert("X-Custom".to_string(), "value\r\nEvil: header".to_string());
            req.headers = Some(headers);
            assert!(validate_mcp_connector(&req).unwrap_err().contains("CR/LF"));
        }

        #[test]
        fn rejects_cwd_with_dotdot() {
            let mut req = stdio_req("npx", vec!["-y", "some-pkg"]);
            req.cwd = Some("/home/user/../etc".to_string());
            assert!(validate_mcp_connector(&req).unwrap_err().contains(".."));
        }

        #[test]
        fn rejects_cwd_relative() {
            let mut req = stdio_req("npx", vec!["-y", "some-pkg"]);
            req.cwd = Some("relative/path".to_string());
            assert!(validate_mcp_connector(&req).unwrap_err().contains("absolute path"));
        }

        #[test]
        fn accepts_cwd_absolute() {
            let mut req = stdio_req("npx", vec!["-y", "some-pkg"]);
            req.cwd = Some("/home/user/workspace".to_string());
            assert!(validate_mcp_connector(&req).is_ok());
        }

        #[test]
        fn rejects_stdio_without_command() {
            let mut req = stdio_req("npx", vec![]);
            req.command = None;
            assert!(validate_mcp_connector(&req).unwrap_err().contains("requires 'command'"));
        }

        #[test]
        fn rejects_sse_without_url() {
            let mut req = sse_req("https://example.com/mcp");
            req.url = None;
            assert!(validate_mcp_connector(&req).unwrap_err().contains("requires 'url'"));
        }

        #[test]
        fn rejects_invalid_transport() {
            let mut req = stdio_req("npx", vec![]);
            req.transport = "websocket".to_string();
            assert!(validate_mcp_connector(&req).unwrap_err().contains("Invalid transport"));
        }
    }
}
