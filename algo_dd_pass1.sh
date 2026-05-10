#!/bin/bash
# algo_dd_pass1: fast sweep. Find big zones, skip fast.
#
# Philosophy: Pass 1 finds the BIG picture. Batch reads only.
# Hit error → mark batch as bad → skip ahead aggressively.
# No bpt=1, no retry. Save precision for Pass 2+.
#
# Skip doubles: 32 → 64 → 128 → ... → 8192 sectors
# Reset skip on first OK read after a skip.

TOOL=/Users/mjackson/Developer/freemkv/freemkv-tools/target/debug/freemkv-tools
DEVICE=${1:-/dev/disk6}
START=${2:-0}
COUNT=${3:-41288704}
BPT=${4:-32}
SKIP_INITIAL=${5:-32}
SKIP_MAX=${6:-8192}

pos=$START
end=$((START + COUNT))
skip=$SKIP_INITIAL
total_ok=0
total_err=0
total_skipped=0
total_bytes_ok=0
t0=$(date +%s)
prev_result=""

echo "# pass1 device=$DEVICE start=$START count=$COUNT bpt=$BPT skip_init=$SKIP_INITIAL skip_max=$SKIP_MAX"
echo "# lba,count,result,skip"

while [ $pos -lt $end ]; do
    remaining=$((end - pos))
    count=$(( BPT < remaining ? BPT : remaining ))
    [ $count -le 0 ] && break

    output=$(perl -e "alarm 120; exec \@ARGV" "$TOOL" dd if=$DEVICE skip=$pos count=$count bpt=$count verbose=0 2>&1)
    ok=$(echo "$output" | grep -o 'sectors_ok=[0-9]*' | sed 's/sectors_ok=//')

    if [ "$ok" = "$count" ]; then
        total_ok=$((total_ok + count))
        total_bytes_ok=$((total_bytes_ok + count * 2048))
        echo "$pos,$count,OK,0"

        # If previous was error and we just recovered, don't skip
        skip=$SKIP_INITIAL

        pos=$((pos + count))
    else
        total_err=$((total_err + count))
        echo "$pos,$count,ERR,$skip"

        pos=$((pos + count))

        # Skip ahead aggressively
        remaining=$((end - pos))
        actual_skip=$(( skip < remaining ? skip : remaining ))

        if [ $actual_skip -gt 0 ]; then
            total_skipped=$((total_skipped + actual_skip))
            echo "# SKIP $actual_skip from $pos" >&2
            pos=$((pos + actual_skip))
            skip=$((skip * 2))
            [ $skip -gt $SKIP_MAX ] && skip=$SKIP_MAX
        fi
    fi
done

t1=$(date +%s)
elapsed=$((t1 - t0))
rate=0
if [ $elapsed -gt 0 ]; then
    rate=$(echo "scale=2; $total_bytes_ok / 1048576 / $elapsed" | bc 2>/dev/null || echo "0")
fi

echo ""
echo "# PASS1_SUMMARY ok_sectors=$total_ok err_sectors=$total_err skipped_sectors=$total_skipped elapsed=${elapsed}s rate=${rate}MB/s"
echo "# bad + skipped = $((total_err + total_skipped)) sectors = ~$(( (total_err + total_skipped) * 2048 / 1048576 )) MB"
