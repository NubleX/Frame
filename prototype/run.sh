#!/usr/bin/env bash
# Starts the barebone browser. The window owns the gate and the VM, so closing
# it, or anything else that ends it, burns the compartment.
# Usage: run.sh <work-dir> <url> [engine flags]
work=$1 url=$2; shift 2
exec "$work/target/release/viewer" "$work" "$url" "$@"
