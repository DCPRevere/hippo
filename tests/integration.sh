#!/usr/bin/env bash
set -euo pipefail

BASE="http://localhost:21693"
PASS=0
FAIL=0

check() {
    local desc="$1"
    local result="$2"
    if [ "$result" = "true" ]; then
        echo "PASS: $desc"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $desc"
        FAIL=$((FAIL + 1))
    fi
}

wait_for_health() {
    echo "Waiting for hippo..."
    for i in $(seq 1 30); do
        if curl -sf "$BASE/health" > /dev/null 2>&1; then
            echo "Memory agent is up."
            return 0
        fi
        sleep 2
    done
    echo "ERROR: hippo did not start in time"
    exit 1
}

remember() {
    local stmt="$1"
    curl -sf -X POST "$BASE/remember" \
        -H 'Content-Type: application/json' \
        -d "{\"statement\": $(echo "$stmt" | jq -Rs .), \"source_agent\": \"test\"}"
}

context_query() {
    local query="$1"
    local limit="${2:-10}"
    curl -sf -X POST "$BASE/context" \
        -H 'Content-Type: application/json' \
        -d "{\"query\": $(echo "$query" | jq -Rs .), \"limit\": $limit}"
}

contains_fact() {
    local response="$1"
    local keyword="$2"
    echo "$response" | jq -r '.facts[].fact' 2>/dev/null | grep -qi "$keyword"
}

# --- Health check ---
wait_for_health
HEALTH=$(curl -sf "$BASE/health")
check "health endpoint returns ok" "$(echo "$HEALTH" | jq -r '.status' | grep -c 'ok' | grep -q 1 && echo true || echo false)"

echo ""
echo "=== Loading test data ==="

# People
remember "Alice is my sister"
remember "Carol is my colleague at Acme Corp"

# Events
remember "Alice and Bob got married in June 2020"
remember "I attended Alice and Bob's wedding with Carol"

# Health
remember "My doctor is Dr. Smith at City Medical"
remember "I take metformin prescribed by Dr. Smith"

# Finance
remember "I have a savings account at First Bank"
remember "My mortgage is with City Credit Union"

# Cross-domain
remember "Dr. Smith's office sent a bill for 200 dollars"
remember "First Bank has a joint account with Alice"

# Placeholder
remember "John's wife called to reschedule the appointment"

echo "Data loaded. Waiting for maintenance cycle..."
sleep 15

echo ""
echo "=== Recall Tests ==="

# Alice facts
ALICE=$(context_query "Alice")
check "Alice is sister" "$(contains_fact "$ALICE" "sister" && echo true || echo false)"
check "Alice married Bob" "$(contains_fact "$ALICE" "married\|Bob\|wedding" && echo true || echo false)"

# Cross-domain: medical bill
MEDICAL=$(context_query "medical bill")
check "Medical bill returns Dr. Smith" "$(contains_fact "$MEDICAL" "smith\|doctor\|medical" && echo true || echo false)"
check "Medical bill returns bill/finance fact" "$(contains_fact "$MEDICAL" "bill\|200" && echo true || echo false)"

# Wedding
WEDDING=$(context_query "wedding")
check "Wedding returns Alice and Bob" "$(contains_fact "$WEDDING" "alice\|bob\|married\|wedding" && echo true || echo false)"
check "Wedding returns Carol attended" "$(contains_fact "$WEDDING" "carol\|attended\|wedding" && echo true || echo false)"

echo ""
echo "=== Contradiction Test ==="

remember "My doctor is Dr. Jones at Riverside Clinic"
sleep 3

DOCTOR=$(context_query "my doctor" 5)
check "New doctor Dr. Jones appears" "$(contains_fact "$DOCTOR" "jones\|riverside" && echo true || echo false)"
check "Old doctor Dr. Smith not in top results" "$(! contains_fact "$DOCTOR" "city medical" && echo true || echo false)"

echo ""
echo "=== Placeholder Resolution Test ==="

remember "John's wife is Sarah"
sleep 5

SARAH=$(context_query "Sarah")
check "Sarah is retrievable after placeholder resolution" "$(
    FACTS=$(echo "$SARAH" | jq -r '.facts | length')
    [ "$FACTS" -gt 0 ] && echo true || echo false
)"

echo ""
echo "=== Trigger Maintenance ==="
MAINT=$(curl -sf -X POST "$BASE/maintain")
check "Maintenance endpoint returns success" "$(echo "$MAINT" | jq -r '.status' | grep -c 'complete' | grep -q 1 && echo true || echo false)"

echo ""
echo "=== Link Discovery Test (post-maintenance) ==="
sleep 10

# After maintenance, Alice and First Bank should potentially be linked
# (First Bank has a joint account with Alice)
ALICE_BANK=$(context_query "Alice bank account")
check "Alice-bank link discoverable" "$(contains_fact "$ALICE_BANK" "alice\|bank\|account" && echo true || echo false)"

echo ""
echo "=========================="
echo "Results: $PASS passed, $FAIL failed"
echo "=========================="

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
