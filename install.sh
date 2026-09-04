#!/usr/bin/env bash
set -euo pipefail
umask 077

readonly repository="VitorHolandaI/perfo"
readonly version="v0.1.5"
readonly install_dir="${PERFO_INSTALL_DIR:-$HOME/.local/bin}"
readonly asset="perfo-linux-x86_64.tar.gz"
readonly expected_sha256="a721098e65d67c16358e941f61e379ba974ab148e6cc86cf47f2b8e178f681bf"

fail() {
    printf 'perfo installer: %s\n' "$1" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

case "$(uname -s)" in
    Linux) ;;
    *) fail "unsupported operating system: $(uname -s) (expected Linux)" ;;
esac

case "$(uname -m)" in
    x86_64|amd64) ;;
    *) fail "unsupported architecture: $(uname -m) (available release: x86_64)" ;;
esac

require_command curl
require_command install
require_command sha256sum
require_command tar

readonly release_url="https://github.com/$repository/releases/download/$version"

temporary_dir="$(mktemp -d)"
trap 'rm -rf "$temporary_dir"' EXIT

curl --fail --location --silent --show-error \
    --proto '=https' --tlsv1.2 --retry 3 --connect-timeout 10 --max-time 120 \
    "$release_url/$asset" \
    --output "$temporary_dir/$asset"

(cd "$temporary_dir" && printf '%s  %s\n' "$expected_sha256" "$asset" | sha256sum --check --strict -)
mapfile -t members < <(tar --list --file "$temporary_dir/$asset")
[[ "${#members[@]}" -eq 1 && "${members[0]}" == "perfo" ]] || \
    fail "release archive contains unexpected files (expected only perfo)"
tar --extract --gzip --no-same-owner --no-same-permissions \
    --file "$temporary_dir/$asset" --directory "$temporary_dir"
[[ -f "$temporary_dir/perfo" && ! -L "$temporary_dir/perfo" && -x "$temporary_dir/perfo" ]] || \
    fail "release archive does not contain a regular executable perfo"

mkdir -p "$install_dir"
install --mode 0755 "$temporary_dir/perfo" "$install_dir/perfo"
printf 'Installed perfo at %s\n' "$install_dir/perfo"

case ":${PATH}:" in
    *":$install_dir:"*) ;;
    *) printf 'Add this directory to PATH: %s\n' "$install_dir" ;;
esac
