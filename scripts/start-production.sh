#!/bin/bash
# Start the log purger with production settings
# This script loads credentials from .env and runs with optimal settings

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}═══════════════════════════════════════════${NC}"
echo -e "${BLUE}    LOG SDK PURGER - PRODUCTION MODE${NC}"
echo -e "${BLUE}═══════════════════════════════════════════${NC}"

# Load environment variables
if [ -f .env ]; then
    source .env
    echo -e "${GREEN}✓ Environment loaded${NC}"
else
    echo -e "${RED}✗ Error: .env file not found!${NC}"
    echo "Please create .env file with DATABASE_URL"
    exit 1
fi

# Parse DATABASE_URL for display
if [[ $DATABASE_URL =~ mysql://([^:]+):([^@]+)@([^:]+):([^/]+)/(.+) ]]; then
    DB_USER="${BASH_REMATCH[1]}"
    DB_HOST="${BASH_REMATCH[3]}"
    DB_PORT="${BASH_REMATCH[4]}"
    DB_NAME="${BASH_REMATCH[5]}"

    echo -e "${GREEN}Configuration:${NC}"
    echo "  Server:   $DB_HOST:$DB_PORT"
    echo "  Database: $DB_NAME"
    echo "  Table:    log_sdk"
    echo "  User:     $DB_USER"
fi

# Check if running in dry-run mode
if [ "$1" == "--dry-run" ]; then
    echo -e "${YELLOW}⚠️  Running in DRY-RUN mode (no actual deletions)${NC}"
    DRY_RUN="--dry-run"
else
    echo -e "${RED}⚠️  Running in PRODUCTION mode (will delete data!)${NC}"
    echo -e "${YELLOW}Press Ctrl+C within 5 seconds to cancel...${NC}"
    sleep 5
    DRY_RUN=""
fi

# Build in release mode if not already built
if [ ! -f "target/release/log-sdk-purger" ]; then
    echo -e "${YELLOW}Building release binary...${NC}"
    cargo build --release
fi

# Production settings
STRATEGY="adaptive"           # Adaptive strategy for optimal performance
BATCH_SIZE=500                # Safe batch size to avoid locks
SLEEP_MS=200                  # Sleep between batches (milliseconds)
RETENTION_DAYS=90             # Keep 90 days of data
MAX_CONNECTIONS=30            # Throttle if too many connections
METRICS_PORT=9090             # Prometheus metrics port

echo ""
echo -e "${GREEN}Starting with settings:${NC}"
echo "  Strategy:        $STRATEGY"
echo "  Batch Size:      $BATCH_SIZE rows"
echo "  Sleep Time:      $SLEEP_MS ms"
echo "  Retention:       $RETENTION_DAYS days"
echo "  Max Connections: $MAX_CONNECTIONS"
echo "  Metrics Port:    $METRICS_PORT"
echo ""

# Run the purger
echo -e "${GREEN}Starting purger...${NC}"
echo "───────────────────────────────────────────"

./target/release/log-sdk-purger \
    --strategy "$STRATEGY" \
    --batch-size "$BATCH_SIZE" \
    --sleep-ms "$SLEEP_MS" \
    --retention-days "$RETENTION_DAYS" \
    --max-connections "$MAX_CONNECTIONS" \
    --metrics-port "$METRICS_PORT" \
    --progress \
    $DRY_RUN

EXIT_CODE=$?

if [ $EXIT_CODE -eq 0 ]; then
    echo ""
    echo -e "${GREEN}✅ Purger completed successfully!${NC}"
else
    echo ""
    echo -e "${RED}✗ Purger exited with error code $EXIT_CODE${NC}"
    exit $EXIT_CODE
fi