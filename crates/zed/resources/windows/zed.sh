#!/usr/bin/env sh

if [ "$ZED_WSL_DEBUG_INFO" = true ]; then
	set -x
fi

ZED_PATH="$(dirname "$(realpath "$0")")"

"$ZED_PATH/zed.exe" "$@"
exit $?
