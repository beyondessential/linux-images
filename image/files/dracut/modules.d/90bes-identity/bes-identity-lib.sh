# shellcheck shell=bash
# Shared build-identity readers, sourced by image/build.sh,
# tests/test-image-structure.sh, and (from here) the 90bes-identity dracut
# module. Bash-compatible; no dependency beyond cryptsetup, blkid, and awk,
# all already required wherever this is sourced.

# Extracts the hex digest bytes for LUKS2 digest 0 from `cryptsetup
# luksDump` text output. Not JSON: jq isn't a guaranteed dependency.
luks_digest0() {
    cryptsetup luksDump "$1" | awk '
        /^Digests:/ { section = 1; next }
        section && /^[[:space:]]*[0-9]+:/ {
            id = $0
            sub(/^[[:space:]]*/, "", id)
            sub(/:.*/, "", id)
            cur = id
            capture = 0
            next
        }
        section && cur == "0" && /Digest:/ {
            line = $0
            sub(/.*Digest:[[:space:]]*/, "", line)
            printf "%s", line
            capture = 1
            next
        }
        section && cur == "0" && capture && /^[[:space:]]+[0-9a-f]{2}([[:space:]][0-9a-f]{2})*[[:space:]]*$/ {
            line = $0
            gsub(/^[[:space:]]+/, "", line)
            printf " %s", line
            next
        }
        { if (capture) capture = 0 }
    ' | tr -d ' \t\n'
}
