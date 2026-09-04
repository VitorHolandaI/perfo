#!/usr/bin/env bash
set -euo pipefail

readonly repository="VitorHolandaI/perfo"
readonly version="${PERFO_VERSION:-latest}"
readonly install_dir="${PERFO_INSTALL_DIR:-$HOME/.local/bin}"
readonly asset="perfo-linux-x86_64.tar.gz"
readonly checksum="perfo-linux-x86_64.sha256"

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

[[ "$version" =~ ^[A-Za-z0-9._-]+$ ]] || fail "invalid PERFO_VERSION: $version"

require_command curl
require_command install
require_command sha256sum
require_command tar

if [[ "$version" == latest ]]; then
    release_url="https://github.com/$repository/releases/latest/download"
else
    release_url="https://github.com/$repository/releases/download/$version"
fi

temporary_dir="$(mktemp -d)"
trap 'rm -rf "$temporary_dir"' EXIT

curl --fail --location --silent --show-error \
    "$release_url/$asset" \
    --output "$temporary_dir/$asset"
curl --fail --location --silent --show-error \
    "$release_url/$checksum" \
    --output "$temporary_dir/$checksum"

(cd "$temporary_dir" && sha256sum --check "$checksum")
tar --extract --gzip --file "$temporary_dir/$asset" --directory "$temporary_dir"
[[ -x "$temporary_dir/perfo" ]] || fail "release archive does not contain an executable perfo"

mkdir -p "$install_dir"
install --mode 0755 "$temporary_dir/perfo" "$install_dir/perfo"
printf 'Installed perfo at %s\n' "$install_dir/perfo"

case ":${PATH}:" in
    *":$install_dir:"*) ;;
    *) printf 'Add this directory to PATH: %s\n' "$install_dir" ;;
esac
