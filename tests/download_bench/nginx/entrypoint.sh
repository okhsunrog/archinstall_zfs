#!/bin/sh
# Apply optional tc qdisc rules from env vars, then exec nginx.
#
# Env vars:
#   RATE_LIMIT  e.g. "10mbit", "500kbit". Empty = no rate limit.
#   NETEM       e.g. "loss 30%", "delay 100ms". Empty = no netem.
#
# Both can be combined — netem is layered above tbf when both are set.

set -eu

# tc isn't in the base nginx alpine image; install on first run.
if ! command -v tc >/dev/null 2>&1; then
    apk add --no-cache iproute2 >/dev/null 2>&1 || true
fi

DEV=eth0

# Wipe any pre-existing qdisc (idempotent across container restarts)
tc qdisc del dev "$DEV" root 2>/dev/null || true

if [ -n "${RATE_LIMIT:-}" ] && [ -n "${NETEM:-}" ]; then
    # Both rate limit and netem: tbf as parent, netem as child
    tc qdisc add dev "$DEV" root handle 1: tbf \
        rate "$RATE_LIMIT" burst 32kbit latency 50ms
    tc qdisc add dev "$DEV" parent 1:1 handle 10: netem $NETEM
    echo "tc: rate=$RATE_LIMIT netem=$NETEM"
elif [ -n "${RATE_LIMIT:-}" ]; then
    tc qdisc add dev "$DEV" root tbf \
        rate "$RATE_LIMIT" burst 32kbit latency 50ms
    echo "tc: rate=$RATE_LIMIT"
elif [ -n "${NETEM:-}" ]; then
    tc qdisc add dev "$DEV" root netem $NETEM
    echo "tc: netem=$NETEM"
else
    echo "tc: no shaping"
fi

exec nginx -g 'daemon off;'
