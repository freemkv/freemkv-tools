#!/bin/bash
# Pass 1 algo tester. Batch reads only, aggressive skip on error.
# No bpt reduction, no retry, no pause. Just map the zones fast.
#
# On error: mark batch NonTrimmed → skip (doubling).
# On OK after skip: reset skip counter.
#
# Args: device start_sector count bpt skip_initial skip_max

TOOL=/Users/mjackson/Developer/freemkv/freemkv-tools/target/debug/freemkv-tools
DEV=${1:-/dev/disk6}
START=${2:-0}
COUNT=${3:-41288704}
BPT=${4:-32}
SKIP_INIT=${5:-32}
SKIP_MAX=${6:-8192}

pos=$START
end=$((START + COUNT))
skip=$SKIP_INIT
total_ok=0
total_err=0
total_skipped=0
bad_starts=()
bad_ends=()
t0=$(date +%s)

# State tracking
in_bad_zone=0
bad_zone_start=0

echo "# pass1 dev=$DEV start=$START count=$COUNT bpt=$BPT skip_init=$SKIP_INIT skip_max=$SKIP_MAX"

while [ $pos -lt $end ]; do
    remaining=$((end - pos))
    cnt=$(( BPT < remaining ? BPT : remaining ))
    [ $cnt -le 0 ] && break

    output=$(perl -e "alarm 60; exec \@ARGV" "$TOOL" dd if=$DEV skip=$pos count=$cnt bpt=$cnt verbose=0 2>&1)
    ok=$(echo "$output" | grep -o 'sectors_ok=[0-9]*' | sed 's/sectors_ok=//')

    if [ "$ok" = "$cnt" ]; then
        total_ok=$((total_ok + cnt))
        
        if [ $in_bad_zone -eq 1 ]; then
            echo "  BAD_ZONE: $bad_zone_start .. $((pos - 1))"
            in_bad_zone=0
        fi
        
        pos=$((pos + cnt))
        skip=$SKIP_INIT
    else
        total_err=$((total_err + cnt))
        
        if [ $in_bad_zone -eq 0 ]; then
            bad_zone_start=$pos
            in_bad_zone=1
        fi
        
        # Mark this batch as bad, advance past it
        pos=$((pos + cnt))
        
        # Skip ahead
        remaining=$((end - pos))
        actual_skip=$(( skip < remaining ? skip : remaining ))
        if [ $actual_skip -gt 0 ]; then
            total_skipped=$((total_skipped + actual_skip))
            pos=$((pos + actual_skip))
            skip=$((skip * 2))
            [ $skip -gt $SKIP_MAX ] && skip=$SKIP_MAX
        fi
    fi
done

if [ $in_bad_zone -eq 1 ]; then
    echo "  BAD_ZONE: $bad_zone_start .. $((end - 1))"
fi

t1=$(date +%s)
elapsed=$((t1 - t0))
total_bad=$((total_err + total_skipped))
echo ""
echo "# RESULT: ok=$total_ok err=$total_err skipped=$total_skipped total_bad=$total_bad elapsed=${elapsed}s"
echo "# Bad sectors: ~$total_bad = ~$((total_bad / 512)) MB of ~$((COUNT / 512)) MB disc"
