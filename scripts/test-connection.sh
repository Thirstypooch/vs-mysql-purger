#!/bin/bash
# Test database connection to production server
# This verifies credentials and network connectivity before running the purger

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}Testing MySQL connection to production server...${NC}"
echo "================================================"

# Load environment variables
if [ -f .env ]; then
    source .env
    echo -e "${GREEN}✓ Loaded .env file${NC}"
else
    echo -e "${RED}✗ Error: .env file not found!${NC}"
    echo "Please create .env file with DATABASE_URL"
    exit 1
fi

# Parse DATABASE_URL
# Format: mysql://username:password@host:port/database
if [[ $DATABASE_URL =~ mysql://([^:]+):([^@]+)@([^:]+):([^/]+)/(.+) ]]; then
    DB_USER="${BASH_REMATCH[1]}"
    DB_PASS="${BASH_REMATCH[2]}"
    DB_HOST="${BASH_REMATCH[3]}"
    DB_PORT="${BASH_REMATCH[4]}"
    DB_NAME="${BASH_REMATCH[5]}"
else
    echo -e "${RED}✗ Error: Invalid DATABASE_URL format${NC}"
    exit 1
fi

echo "Server: $DB_HOST:$DB_PORT"
echo "Database: $DB_NAME"
echo "User: $DB_USER"
echo "Table: log_sdk"
echo ""

# Test connection and get table info
echo -e "${YELLOW}Testing connection...${NC}"
mysql -h "$DB_HOST" -P "$DB_PORT" -u "$DB_USER" -p"$DB_PASS" "$DB_NAME" <<EOF 2>/dev/null && CONNECTION_OK=1 || CONNECTION_OK=0
    SELECT
        COUNT(*) as total_rows,
        MIN(fecha) as oldest_record,
        MAX(fecha) as newest_record,
        COUNT(CASE WHEN fecha < DATE_SUB(NOW(), INTERVAL 90 DAY) THEN 1 END) as rows_to_purge
    FROM log_sdk;
EOF

if [ $CONNECTION_OK -eq 1 ]; then
    echo -e "${GREEN}✅ Connection successful!${NC}"
    echo ""

    # Get detailed table statistics
    echo -e "${YELLOW}Table Statistics:${NC}"
    mysql -h "$DB_HOST" -P "$DB_PORT" -u "$DB_USER" -p"$DB_PASS" "$DB_NAME" -e "
        SELECT
            COUNT(*) as 'Total Rows',
            COUNT(CASE WHEN fecha < DATE_SUB(NOW(), INTERVAL 90 DAY) THEN 1 END) as 'Rows to Purge (>90 days)',
            ROUND(SUM(LENGTH(parametros) + LENGTH(output)) / 1024 / 1024 / 1024, 2) as 'Total Data (GB)',
            MIN(fecha) as 'Oldest Record',
            MAX(fecha) as 'Newest Record'
        FROM log_sdk;
    " 2>/dev/null

    echo ""
    echo -e "${GREEN}Ready to run purger!${NC}"
    echo ""
    echo "To run in dry-run mode (no deletions):"
    echo "  cargo run -- --dry-run --progress"
    echo ""
    echo "To run actual purge:"
    echo "  cargo run -- --strategy adaptive --batch-size 500 --progress"
else
    echo -e "${RED}✗ Connection failed!${NC}"
    echo "Please check:"
    echo "  1. Server is accessible at $DB_HOST:$DB_PORT"
    echo "  2. Credentials are correct"
    echo "  3. Database '$DB_NAME' exists"
    echo "  4. Table 'log_sdk' exists"
    exit 1
fi