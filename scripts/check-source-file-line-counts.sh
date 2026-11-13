#!/usr/bin/env bash
set -euo pipefail

# 800 lines is a guideline, not a hard limit (owner decision 2026-11-13,
# relaxing the 2026-11-05 rule). The point is to avoid God files, which is a
# judgement about responsibilities, not length. Files at or over the guideline
# are listed for review (as GitHub warnings in CI); the check never fails.
readonly GUIDELINE_LINES=800

checked_files=0
over_guideline=()

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

    if (( line_count >= GUIDELINE_LINES )); then
        over_guideline+=("${line_count} ${file}")
    fi
done < <(git ls-files -z)

if (( ${#over_guideline[@]} > 0 )); then
    printf 'Checked %d tracked source files; %d are at or over the %d-line guideline.\n' \
        "${checked_files}" "${#over_guideline[@]}" "${GUIDELINE_LINES}"
    printf 'Review each for mixed responsibilities (God files); length alone is not a failure.\n'
    while IFS= read -r entry; do
        printf '  %s\n' "${entry}"
        if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
            printf '::warning file=%s::%s lines, over the %d-line guideline; check it is not a God file.\n' \
                "${entry#* }" "${entry%% *}" "${GUIDELINE_LINES}"
        fi
    done < <(printf '%s\n' "${over_guideline[@]}" | sort -nr)
    exit 0
fi

printf 'Checked %d tracked source files; all are below the %d-line guideline.\n' \
    "${checked_files}" \
    "${GUIDELINE_LINES}"
