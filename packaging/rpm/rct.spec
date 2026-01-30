Name:           rct
Version:        0.1.0
Release:        1%{?dist}
Summary:        High-performance Rust CLI for Claude API

License:        MIT OR Apache-2.0
URL:            https://github.com/postrv/rct
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo >= 1.75
BuildRequires:  rust >= 1.75

%description
RCT (Rust Claude Terminal) is a high-performance command-line interface
for interacting with the Anthropic Claude API. It features streaming
responses, syntax highlighting, tool execution, and a modern TUI.

Features:
- Streaming API responses with real-time display
- Syntax highlighting for code blocks
- Built-in tools (bash, file operations, search)
- MCP protocol support
- Plugin and skill system
- Session persistence

%prep
%autosetup

%build
cargo build --release --locked

%install
install -D -m 755 target/release/rct %{buildroot}%{_bindir}/rct

%check
cargo test --release --locked

%files
%license LICENSE-MIT LICENSE-APACHE
%doc README.md
%{_bindir}/rct

%changelog
* Thu Jan 30 2026 RCT Developers <rct@example.com> - 0.1.0-1
- Initial release
