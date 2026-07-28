#!/usr/bin/env bash
set -euo pipefail

readonly MAX_EXCLUSIVE_LINES=800

checked_files=0
violations=()

while IFS= read -r -d '' file; do
    case "${file}" in
        *.c|*.cc|*.cpp|*.cxx|*.h|*.hh|*.hpp|*.hxx|*.java|*.js|*.jsx|*.kt|*.kts|*.py|*.rs|*.sh|*.swift|*.ts|*.tsx|*.css|*.scss|*.html)
            ;;
        *)
            continue
            ;;
    esac

    line_count=$(awk 'END { print NR }' "${file}")
    ((checked_files += 1))

    if (( line_count >= MAX_EXCLUSIVE_LINES )); then
        violations+=("${line_count} ${file}")
    fi
done < <(git ls-files -z)

if (( ${#violations[@]} > 0 )); then
    printf 'Tracked source files must remain below %d physical lines.\n' "${MAX_EXCLUSIVE_LINES}" >&2
    printf '%s\n' "${violations[@]}" | sort -nr >&2
    exit 1
fi

printf 'Checked %d tracked source files; all are below %d physical lines.\n' \
    "${checked_files}" \
    "${MAX_EXCLUSIVE_LINES}"
