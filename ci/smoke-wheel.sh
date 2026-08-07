#!/bin/sh
# Smoke-test a freshly built uncompose-project wheel: install it into a clean
# virtualenv and exercise the installed binary the way a user (and the root
# `uncompose` dispatcher) would. Fails loudly on any deviation.
#
# Usage: ci/smoke-wheel.sh <wheel-dir>
#
# Requires `uv` on PATH (provides an isolated venv + pip without ensurepip).
set -eu

wheel_dir="${1:?usage: smoke-wheel.sh <wheel-dir>}"
wheel_dir="$(cd "$wheel_dir" && pwd)"
wheel="$(ls "$wheel_dir"/uncompose_project-*.whl 2>/dev/null | head -n1)"
[ -n "$wheel" ] || { echo "no wheel found in $wheel_dir" >&2; exit 1; }
case "$wheel" in
    *manylinux*) : ;;
    *) echo "wheel is not a manylinux wheel: $wheel" >&2; exit 1 ;;
esac
echo "testing $wheel"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"

uv venv .venv
uv pip install --python .venv/bin/python "$wheel"
bin="$work/.venv/bin/uncompose-project"

# The dispatch contract requires --version and --help to work.
"$bin" --version
"$bin" --help

# A real command takes effect and writes the manifest.
mkdir proj
( cd proj && "$bin" init )
test -f proj/uncompose.project.json

# ADR-0005 root dispatch: `uncompose <sub> <args>` execs `uncompose-<sub> <args>`
# from PATH. Stand up that dispatcher and confirm delegation preserves output
# and exit codes against the installed binary.
mkdir shim
cat > shim/uncompose <<'DISPATCH'
#!/bin/sh
sub="$1"; shift
exec "uncompose-$sub" "$@"
DISPATCH
chmod +x shim/uncompose
export PATH="$work/shim:$work/.venv/bin:$PATH"

# --version delegated is byte-identical to a direct invocation.
delegated="$(uncompose project --version)"
direct="$(uncompose-project --version)"
[ "$delegated" = "$direct" ] || {
    echo "dispatch changed --version output: '$delegated' vs '$direct'" >&2
    exit 1
}

# A delegated command succeeds, and a refusal's non-zero exit is preserved.
mkdir dproj
( cd dproj && uncompose project init )
if ( cd dproj && uncompose project init ); then
    echo "re-init through dispatch should have exited non-zero" >&2
    exit 1
fi

echo "wheel smoke test passed"
