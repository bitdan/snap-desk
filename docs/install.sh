#!/bin/sh

fail() { printf 'Snap Desk: %s\n' "$*" >&2; exit 1; }

# 只接受同一正式版目录下与当前平台匹配的官方附件。
find_asset() {
    manifest=$(curl -fsL --connect-timeout 8 --max-time 20 "$1") || return 1
    row=$(printf '%s\n' "$manifest" | awk -F '\t' -v key="$kind" '$1 == key && NF == 3 { sub(/\r$/, "", $3); printf "%s\t%s\t%s", $1, $2, $3; exit }')
    IFS="$(printf '\t')" read -r asset url digest <<EOF
$row
EOF
    release='https://github.com/bitdan/snap-desk/releases/download/'
    version=${url#"${release}v"}
    version=${version%%/*}
    printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || return 1
    printf '%s\n' "$digest" | grep -Eq '^[0-9a-f]{64}$' || return 1
    [ "$url" = "${release}v${version}/snap-desk-v${version}-${suffix}" ]
}

cleanup() {
    if [ -n "$mounted" ] && ! hdiutil detach "$mounted" -quiet; then
        printf 'Detach the disk image at %s\n' "$mounted" >&2
    else
        find "$work" -depth -delete
    fi
    if [ -n "$stage" ]; then
        # 安装目录和暂存目录位于同一个卷，替换失败后恢复原来的应用。
        if [ -d "$stage/previous.app" ] && [ ! -e "$target" ]; then
            mv "$stage/previous.app" "$target" || { printf 'Previous app saved at %s/previous.app\n' "$stage" >&2; stage=; }
        fi
        if [ -n "$stage" ]; then find "$stage" -depth -delete; fi
    fi
    if [ -n "$lock" ]; then rmdir "$lock"; fi
}

main() {
    case "$(uname -s):$(uname -m):${1-}" in
        Darwin:arm64:) kind=dmg; suffix=macos-arm64.dmg ;;
        Linux:x86_64:) kind=appimage; suffix=linux-x64.AppImage ;;
        Linux:x86_64:--deb) kind=deb; suffix=linux-x64.deb ;;
        *) fail 'Supported targets are Apple silicon macOS and x64 Linux (--deb for Debian / Ubuntu).' ;;
    esac
    [ "$#" -le 1 ] || fail 'Unexpected installer arguments.'
    if [ "$kind" = deb ]; then
        command -v apt >/dev/null && command -v sudo >/dev/null || fail '--deb requires apt and sudo.'
    fi
    if ! find_asset 'https://raw.githubusercontent.com/bitdan/homebrew-snap-desk/HEAD/downloads.tsv'; then
        find_asset 'https://bitdan.github.io/snap-desk/install/downloads.tsv' || fail 'Could not resolve a stable download. See GitHub Releases.'
    fi

    work=$(mktemp -d "${TMPDIR:-/tmp}/snap-desk.XXXXXXXX") || fail 'Could not create a temporary directory.'
    mounted= stage= lock= target=
    trap cleanup 0
    trap 'exit 130' INT
    trap 'exit 143' TERM

    name=${url##*/}
    package="$work/$name"
    curl -fLS --output "$package" "$url" || fail 'Download failed.'
    actual=$(
        cd "$work" || exit 1
        if [ "$kind" = dmg ]; then shasum -a 256 "$name"; else sha256sum "$name"; fi
    ) || fail 'Could not calculate SHA-256.'
    [ "${actual%% *}" = "$digest" ] || fail 'SHA-256 mismatch.'

    case "$kind" in
        dmg)
            directory="$HOME/Applications"
            target="$directory/Snap Desk.app"
            mkdir -p "$directory" || fail 'Could not create ~/Applications.'
            [ ! -L "$target" ] || fail 'The destination is a symbolic link.'
            lock_path="$directory/.snap-desk-install.lock"
            mkdir "$lock_path" || fail 'An installation is already in progress.'
            lock=$lock_path
            stage=$(mktemp -d "$directory/.snap-desk.XXXXXXXX") || fail 'Could not stage Snap Desk.app.'
            mkdir "$work/mount" || fail 'Could not prepare the disk image mount point.'
            hdiutil attach "$package" -quiet -nobrowse -readonly -mountpoint "$work/mount" || fail 'Could not mount the disk image.'
            mounted="$work/mount"
            [ -d "$mounted/Snap Desk.app" ] || fail 'Snap Desk.app is missing from the disk image.'
            ditto "$mounted/Snap Desk.app" "$stage/next.app" || fail 'Could not copy Snap Desk.app.'
            if [ -e "$target" ]; then
                [ -d "$target" ] && mv "$target" "$stage/previous.app" || fail 'Could not move the previous installation.'
            fi
            mv "$stage/next.app" "$target" || fail 'Could not replace Snap Desk.app.'
            printf 'Installed at %s\n' "$target"
            ;;
        appimage)
            directory="$HOME/.local/bin"
            target="$directory/snap-desk.AppImage"
            mkdir -p "$directory" || fail 'Could not create ~/.local/bin.'
            stage=$(mktemp -d "$directory/.snap-desk.XXXXXXXX") || fail 'Could not stage the AppImage.'
            install -m 755 "$package" "$stage/snap-desk.AppImage" || fail 'Could not stage the AppImage.'
            mv -fT "$stage/snap-desk.AppImage" "$target" || fail 'Could not install the AppImage.'
            printf 'Installed at %s\nLaunch with: "%s"\n' "$target" "$target"
            ;;
        deb)
            chmod 755 "$work" && chmod 644 "$package" || fail 'Could not grant apt read access to the package.'
            # 安装脚本从管道读取，apt 的确认输入仍来自当前终端。
            sudo apt install "$package" </dev/tty || fail 'Package installation did not complete.'
            ;;
    esac
}

main "$@"
