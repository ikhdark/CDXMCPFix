#!/usr/bin/env sh
set -eu

VERSION="${CDXCORE_VERSION:-v0.1.5}"
INSTALL_DIR="${CDXCORE_INSTALL_DIR:-}"
SKIP_CODEX_SETUP="${CDXCORE_SKIP_CODEX_SETUP:-0}"
NO_PATH_UPDATE="${CDXCORE_NO_PATH_UPDATE:-0}"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || { echo "--version requires a value" >&2; exit 2; }
            VERSION="$2"
            shift 2
            ;;
        --install-dir)
            [ "$#" -ge 2 ] || { echo "--install-dir requires a value" >&2; exit 2; }
            INSTALL_DIR="$2"
            shift 2
            ;;
        --skip-codex-setup)
            SKIP_CODEX_SETUP=1
            shift
            ;;
        --no-path-update)
            NO_PATH_UPDATE=1
            shift
            ;;
        *)
            echo "unknown option: $1" >&2
            exit 2
            ;;
    esac
done


if [ -z "$INSTALL_DIR" ]; then
    if [ -z "${HOME:-}" ]; then
        echo "HOME is not set; pass --install-dir." >&2
        exit 2
    fi
    INSTALL_DIR="$HOME/.local/bin"
fi

normalize_install_dir() {
    raw_dir="$1"
    case "$raw_dir" in
        ""|"/"|"/."|"//"|"///")
            echo "install directory must not be a filesystem root" >&2
            exit 2
            ;;
    esac

    parent_dir="$(dirname "$raw_dir")"
    leaf_dir="$(basename "$raw_dir")"
    case "$leaf_dir" in
        ""|"."|".."|"/")
            echo "install directory must resolve to a named directory, not a filesystem root" >&2
            exit 2
            ;;
    esac

    mkdir -p "$parent_dir"
    parent_dir="$(cd "$parent_dir" && pwd -P)"
    INSTALL_DIR="$parent_dir/$leaf_dir"
}

normalize_install_dir "$INSTALL_DIR"
case "$INSTALL_DIR" in
    ""|"/"|"/."|"//"|"///")
        echo "install directory must not be a filesystem root" >&2
        exit 2
        ;;
esac

os="$(uname -s)"
arch="$(uname -m)"
case "$os:$arch" in
    Linux:x86_64|Linux:amd64)
        target="x86_64-unknown-linux-gnu"
        ;;
    Darwin:arm64|Darwin:aarch64)
        target="aarch64-apple-darwin"
        ;;
    Darwin:x86_64|Darwin:amd64)
        echo "macOS Intel is not published for this CDXCore release." >&2
        exit 2
        ;;
    *)
        echo "unsupported platform: $os $arch" >&2
        exit 2
        ;;
esac

repo="ikhdark/CDXCore"
asset_name="cdxcore-$VERSION-$target.tar.gz"
release_base="https://github.com/$repo/releases/download/$VERSION"
archive_url="$release_base/$asset_name"
sums_url="$release_base/SHA256SUMS.txt"
tmp_root="$(mktemp -d 2>/dev/null || mktemp -d -t cdxcore-install)"
archive_path="$tmp_root/$asset_name"
sums_path="$tmp_root/SHA256SUMS.txt"
extract_dir="$tmp_root/extract"

cleanup() {
    rm -rf "$tmp_root"
}
trap cleanup EXIT HUP INT TERM

download() {
    uri="$1"
    out="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$uri" -o "$out"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$out" "$uri"
    else
        echo "curl or wget is required to download CDXCore." >&2
        exit 2
    fi
}

sha256_file() {
    path="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
    else
        echo "sha256sum or shasum is required to verify CDXCore." >&2
        exit 2
    fi
}

shell_quote() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

path_line_for_profile() {
    quoted_install_dir="$(shell_quote "$INSTALL_DIR")"
    printf 'export PATH=%s:"$PATH"\n' "$quoted_install_dir"
}

add_path_to_profile() {
    [ "$NO_PATH_UPDATE" = "1" ] && return 0
    [ -z "${HOME:-}" ] && return 0
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) return 0 ;;
    esac

    shell_name="$(basename "${SHELL:-}")"
    case "$shell_name" in
        zsh) profile="$HOME/.zshrc" ;;
        bash) profile="$HOME/.bashrc" ;;
        *) profile="$HOME/.profile" ;;
    esac

    line="$(path_line_for_profile)"
    if [ -f "$profile" ] && grep -F "$INSTALL_DIR" "$profile" >/dev/null 2>&1; then
        return 0
    fi
    {
        printf '\n# Added by CDXCore installer\n'
        printf '%s' "$line"
    } >> "$profile"
    echo "Added $INSTALL_DIR to PATH in $profile"
}

mkdir -p "$extract_dir"
echo "Downloading $asset_name..."
download "$archive_url" "$archive_path"
download "$sums_url" "$sums_path"

expected_hash="$(awk -v name="$asset_name" '$2 == name { print tolower($1); exit }' "$sums_path")"
if [ -z "$expected_hash" ]; then
    echo "Could not find $asset_name in SHA256SUMS.txt." >&2
    exit 1
fi
actual_hash="$(sha256_file "$archive_path" | tr '[:upper:]' '[:lower:]')"
if [ "$actual_hash" != "$expected_hash" ]; then
    echo "Checksum mismatch for $asset_name. Expected $expected_hash but got $actual_hash." >&2
    exit 1
fi

tar -xzf "$archive_path" -C "$extract_dir"
mkdir -p "$INSTALL_DIR"
cp "$extract_dir/cdxcore" "$INSTALL_DIR/cdxcore"
chmod 0755 "$INSTALL_DIR/cdxcore"
[ -f "$extract_dir/README.md" ] && cp "$extract_dir/README.md" "$INSTALL_DIR/README.md"
[ -f "$extract_dir/LICENSE" ] && cp "$extract_dir/LICENSE" "$INSTALL_DIR/LICENSE"
if [ -d "$extract_dir/schemas" ]; then
    rm -rf "$INSTALL_DIR/schemas"
    cp -R "$extract_dir/schemas" "$INSTALL_DIR/schemas"
fi
if [ -d "$extract_dir/docs" ]; then
    rm -rf "$INSTALL_DIR/docs"
    cp -R "$extract_dir/docs" "$INSTALL_DIR/docs"
fi

add_path_to_profile
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) PATH="$INSTALL_DIR:$PATH"; export PATH ;;
esac

"$INSTALL_DIR/cdxcore" --version

if [ "$SKIP_CODEX_SETUP" != "1" ]; then
    if ! "$INSTALL_DIR/cdxcore" setup codex; then
        echo "Warning: CDXCore was installed, but Codex setup did not complete. Run 'cdxcore setup codex' after Codex is available on PATH." >&2
    fi
fi

echo "Installed CDXCore to $INSTALL_DIR"
echo "Open a new terminal or restart Codex if it does not see the updated PATH."
