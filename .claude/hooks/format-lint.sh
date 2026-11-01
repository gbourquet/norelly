#!/usr/bin/env bash
# Hook PostToolUse (Edit|Write) : formate puis lint le fichier modifié.
# Code de sortie 2 = problème de lint, renvoyé à Claude pour correction.
set -uo pipefail

file=$(jq -r '.tool_input.file_path // empty')
[ -n "$file" ] && [ -f "$file" ] || exit 0

root="${CLAUDE_PROJECT_DIR:-$(git -C "$(dirname "$file")" rev-parse --show-toplevel 2>/dev/null)}"
[ -n "$root" ] || exit 0
case "$file" in "$root"/*) ;; *) exit 0 ;; esac
rel="${file#"$root"/}"

fail() { printf '%s\n' "$1" >&2; exit 2; }

case "$rel" in
  *.rs)
    rustfmt --edition 2024 --config-path "$root/rustfmt.toml" "$file" 2>&1 >/dev/null \
      || fail "rustfmt a échoué sur $rel"
    # Clippy sur le crate qui contient le fichier.
    dir=$(dirname "$file")
    while [ "$dir" != "$root" ] && [ ! -f "$dir/Cargo.toml" ]; do dir=$(dirname "$dir"); done
    pkg=$(sed -n 's/^name *= *"\(.*\)"/\1/p' "$dir/Cargo.toml" | head -1)
    [ -n "$pkg" ] || exit 0
    out=$(cd "$root" && cargo clippy -p "$pkg" --all-targets --quiet --message-format short -- -D warnings 2>&1)
    [ $? -eq 0 ] || fail "clippy ($pkg) :
$(grep -E '^(error|warning)|^[^ ].*\.rs:[0-9]+' <<<"$out" | head -30)"
    ;;
  app/src-tauri/*) ;;
  app/*.ts | app/*.svelte | app/*.js)
    (cd "$root/app" && pnpm exec prettier --write --log-level warn "$file") || fail "prettier a échoué sur $rel"
    out=$(cd "$root/app" && pnpm exec eslint "$file" 2>&1) || fail "eslint :
$(head -30 <<<"$out")"
    ;;
  app/*.css | app/*.json | app/*.html)
    (cd "$root/app" && pnpm exec prettier --write --log-level warn "$file") || fail "prettier a échoué sur $rel"
    ;;
esac
exit 0
