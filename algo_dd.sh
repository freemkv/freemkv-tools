#!/bin/bash
# freemkv-tools dd wrapper: proves out the skip-ahead algorithm
# against a real disc with bad zones.
#
# Logic:
#   - Read bpt sectors at a time, advancing normally on OK
#   - On error: drop to bpt=1, pause error_pause_ms
#   - After skip_threshold consecutive errors: skip ahead (doubling)
#   - After restore_streak consecutive OKs at bpt=1: restore to full bpt
#
# Outputs a mapfile-compatible log of good/bad ranges.

TOOL=/Users/mjackson/Developer/freemkv/freemkv-tools/target/debug/freemkv-tools
DEVICE=${1:-/dev/disk6}
START=${2:-0}
COUNT=${3:-41288704}  # full disc
BPT=${4:-32}
ERROR_PAUSE=${5:-2000}
SKIP_THRESHOLD=${6:-8}
RESTORE_STREAK=${7:-200}

# Algorithm state
pos=$START
end=$((START + COUNT))
bpt_current=$BPT
consecutive_ok=0
consecutive_err=0
skip_power=0
total_ok=0
total_err=0
skip_sectors_total=0
t0=$(date +%s)

echo "# algo_dd device=$DEVICE start=$START count=$COUNT bpt=$BPT pause=${ERROR_PAUSE}ms skip_after=$SKIP_THRESHOLD restore=$RESTORE_STREAK"
echo "# lba,count,result,elapsed_ms,consecutive_err,current_bpt"

while [ $pos -lt $end ]; do
    remaining=$((end - pos))
    count=$(( bpt_current < remaining ? bpt_current : remaining ))
    [ $count -le 0 ] && break

    # Run dd for one CDB
    output=$(perl -e "alarm 120; exec \@ARGV" "$TOOL" dd if=$DEVICE skip=$pos count=$count bpt=$count verbose=0 2>&1)
    ok=$(echo "$output" | grep -o 'sectors_ok=[0-9]*' | sed 's/sectors_ok=//')
    elapsed=$(echo "$output" | grep -o 'elapsed=[0-9.]*s' | sed 's/elapsed=//;s/s//')

    if [ "$ok" = "$count" ]; then
        # OK
        total_ok=$((total_ok + count))
        consecutive_ok=$((consecutive_ok + 1))
        consecutive_err=0
        skip_power=0

        echo "$pos,$count,OK,$elapsed,$consecutive_err,$bpt_current"

        # Batch restore
        if [ $bpt_current -lt $BPT ] && [ $consecutive_ok -ge $RESTORE_STREAK ]; then
            bpt_current=$(( bpt_current * 2 ))
            [ $bpt_current -gt $BPT ] && bpt_current=$BPT
            echo "# BATCH_RESTORE bpt=$bpt_current after $consecutive_ok OK at lba=$pos" >&2
            consecutive_ok=0
        fi

        pos=$((pos + count))
    else
        # ERROR
        total_err=$((total_err + count))
        consecutive_ok=0
        consecutive_err=$((consecutive_err + 1))

        echo "$pos,$count,ERR,$elapsed,$consecutive_err,$bpt_current"

        # Batch reduce
        if [ $bpt_current -gt 1 ]; then
            bpt_current=1
            echo "# BATCH_REDUCE bpt=1 at lba=$pos" >&2
        fi

        pos=$((pos + count))

        # Skip-ahead after threshold
        if [ $consecutive_err -ge $SKIP_THRESHOLD ]; then
            base=32
            skip=$(( base * (2 ** skip_power) ))
            [ $skip -gt 8192 ] && skip=8192
            remaining=$((end - pos))
            [ $skip -gt $remaining ] && skip=$remaining
            
            if [ $skip -gt 0 ]; then
                echo "# SKIP $skip sectors from lba=$pos (consecutive_err=$consecutive_err skip_power=$skip_power)" >&2
                skip_sectors_total=$((skip_sectors_total + skip))
                pos=$((pos + skip))
                skip_power=$((skip_power + 1))
            fi
        fi

        # Pause between errors
        if [ "$ERROR_PAUSE" -gt 0 ]; then
            sleep $(printf '%d.%03d' $((ERROR_PAUSE / 1000)) $((ERROR_PAUSE % 1000)))
        fi
    fi
done

t1=$(date +%s)
elapsed_total=$((t1 - t0))
echo ""
echo "# SUMMARY: ok=$total_ok err=$total_err skipped=$skip_sectors_total pos=$pos end=$end elapsed=${elapsed_total}s"
