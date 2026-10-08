#!/usr/bin/env bash
# Opt-in installer. Does not replace default applications or portal preferences.
set -euo pipefail

task_directory=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
task_project=$(dirname -- "$task_directory")
task_prefix="${HOME}/.local"
task_destdir=""
task_binary="${task_project}/target/release/telorgon-file-explorer"

usage() {
    cat <<'EOF'
Usage: packaging/install.sh [--prefix /absolute/path] [--destdir /staging/path] [--binary /built/executable]

Defaults to ~/.local and target/release/telorgon-file-explorer.
Installs the executable, desktop entry, D-Bus activation files, portal registration,
and a portal preference example. Does not enable/replace portal configuration,
set a default file manager, restart services, or change the running desktop.

--destdir stages files for packaging, while --prefix sets their runtime prefix.
EOF
}

while (($#)); do
    case "$1" in
        --prefix|--destdir|--binary)
            if (($# < 2)); then usage >&2; exit 2; fi
            case "$1" in
                --prefix) task_prefix=$2 ;;
                --destdir) task_destdir=$2 ;;
                --binary) task_binary=$2 ;;
            esac
            shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'Unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
done

# Prefixes become Exec fields. Limit their alphabet to avoid ambiguous desktop
# and D-Bus quoting; input binary and staging paths can still contain spaces.
if [[ ! $task_prefix =~ ^/[A-Za-z0-9_./+-]+$ ]]; then
    printf 'The runtime prefix must be absolute and contain only letters, digits, /, _, ., +, or -.\n' >&2
    exit 2
fi
if [[ -n $task_destdir && $task_destdir != /* ]]; then
    printf 'The staging directory must be an absolute path.\n' >&2
    exit 2
fi
if [[ ! -f $task_binary || ! -x $task_binary ]]; then
    printf 'Built executable not found: %s\nRun cargo build --release first, or supply --binary.\n' "$task_binary" >&2
    exit 1
fi

task_prefix=${task_prefix%/}
task_install_root="${task_destdir}${task_prefix}"
task_executable="${task_prefix}/bin/telorgon-file-explorer"
install -Dm755 -- "$task_binary" "${task_install_root}/bin/telorgon-file-explorer"
install -Dm644 -- "$task_directory/telorgon-file-explorer.portal" "${task_install_root}/share/xdg-desktop-portal/portals/telorgon-file-explorer.portal"
install -Dm644 -- "$task_directory/telorgon-portals.conf.example" "${task_install_root}/share/telorgon-file-explorer/telorgon-portals.conf.example"

render_template() {
    local task_template=$1 task_destination=$2
    mkdir -p -- "$(dirname -- "$task_destination")"
    sed "s|@EXEC@|${task_executable}|g" "$task_template" > "$task_destination"
    chmod 644 -- "$task_destination"
}
render_template "$task_directory/telorgon-file-explorer.desktop.in" "${task_install_root}/share/applications/telorgon-file-explorer.desktop"
render_template "$task_directory/org.freedesktop.impl.portal.desktop.telorgon.FileExplorer.service.in" "${task_install_root}/share/dbus-1/services/org.freedesktop.impl.portal.desktop.telorgon.FileExplorer.service"
render_template "$task_directory/org.freedesktop.FileManager1.service.in" "${task_install_root}/share/dbus-1/services/org.freedesktop.FileManager1.service"

printf 'Installed Telorgon Files under %s\n' "$task_install_root"
printf 'Portal preferences were left unchanged. See README.md for activation and discovery.\n'
