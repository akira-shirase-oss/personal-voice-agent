#!/usr/bin/env bash
# Architecture rules, enforced rather than described:
#  1. voice-domain is pure: its [dependencies] are an allowlist (serde, thiserror), and its
#     source uses no filesystem, network, process, thread, environment or clock API.
#  2. Nothing in the workspace can talk to a network (D-014): no HTTP, TLS or socket crate
#     appears anywhere in Cargo.lock, and no Rust source opens a socket.
set -euo pipefail
cd "$(dirname "$0")/.."
fail=0

deps="$(sed -n '/^\[dependencies\]/,/^\[/p' crates/voice-domain/Cargo.toml \
  | grep -E '^[a-zA-Z0-9_-]+ *[.=]' | sed -E 's/^([a-zA-Z0-9_-]+).*/\1/' || true)"
for dep in $deps; do
  case "$dep" in
    serde|thiserror) ;;
    *) echo "error: voice-domain may depend only on serde and thiserror, found: $dep" >&2; fail=1 ;;
  esac
done

if grep -rnE 'std::(fs|net|process|thread|env)\b|SystemTime|Instant|std::io::(stdin|stdout|stderr)|tokio' crates/voice-domain/src; then
  echo "error: voice-domain source must not use I/O, threads, the environment or a clock" >&2
  fail=1
fi

if grep -nE '^name = "(reqwest|hyper|h2|h3|rustls|native-tls|openssl|ureq|curl|tungstenite|tokio-tungstenite|quinn|isahc|surf|attohttpc)"' Cargo.lock; then
  echo "error: a network crate is in Cargo.lock; this workspace makes no network calls (D-014)" >&2
  fail=1
fi
if grep -rnE 'std::net::|tokio::net::|TcpListener|TcpStream|UdpSocket|UnixListener' crates/*/src; then
  echo "error: no Rust source may open a socket; the service speaks only stdio (D-015)" >&2
  fail=1
fi

[ "$fail" -eq 0 ] || exit 1
echo "architecture ok: voice-domain is pure; no network crate or socket in the workspace"
