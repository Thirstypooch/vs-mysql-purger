#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
NC='\033[0m'

# Configuration
DB_NAME="goals_logs_benchmark"
DB_USER="root"
DB_PASS="password"
DB_HOST="localhost"
BINARY_PATH="./target/release/log-sdk-purger"

echo -e "${BLUE}════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}     LOG SDK PURGER - PERFORMANCE BENCHMARK            ${NC}"
echo -e "${BLUE}════════════════════════════════════════════════════════${NC}"

# Function to run mysql commands
mysql_exec() {
    mysql -u$DB_USER -p$DB_PASS -h$DB_HOST -e "$1" 2>/dev/null
}

# Function to generate test data
generate_test_data() {
    local rows=$1
    local chunk_size=1000

    echo -e "${YELLOW}Generating $rows test rows...${NC}"

    # Create benchmark database if not exists
    mysql_exec "CREATE DATABASE IF NOT EXISTS $DB_NAME;"
    mysql_exec "USE $DB_NAME; DROP TABLE IF EXISTS log_sdk;"

    # Create table with exact structure
    mysql_exec "USE $DB_NAME; CREATE TABLE log_sdk (
        id INT AUTO_INCREMENT,
        fecha DATETIME,
        proceso VARCHAR(100),
        result TINYINT(1),
        descripcion VARCHAR(255),
        registros_input INT,
        registros_output INT,
        parametros MEDIUMTEXT,
        user VARCHAR(20),
        updated DATETIME,
        output MEDIUMTEXT,
        PRIMARY KEY (fecha, id),
        KEY idx_id (id),
        KEY idx_fecha_user_proc (fecha, user, proceso),
        KEY idx_user_fecha (user, fecha),
        KEY idx_proceso_fecha (proceso, fecha)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;"

    # Generate data in chunks
    for ((i=0; i<$rows; i+=$chunk_size)); do
        local remaining=$((rows - i))
        local current_chunk=$((remaining < chunk_size ? remaining : chunk_size))

        # Create SQL file with test data
        cat > /tmp/insert_batch.sql <<EOF
USE $DB_NAME;
INSERT INTO log_sdk (fecha, proceso, result, descripcion, registros_input, registros_output, parametros, user, updated, output) VALUES
EOF

        for ((j=0; j<$current_chunk; j++)); do
            local days_ago=$((RANDOM % 180))  # Random date within last 180 days
            local json_size=$((RANDOM % 10000 + 1000))  # Random JSON size 1KB-10KB

            if [ $j -gt 0 ]; then
                echo -n "," >> /tmp/insert_batch.sql
            fi

            # Generate realistic JSON data
            cat >> /tmp/insert_batch.sql <<EOF
(
    DATE_SUB(NOW(), INTERVAL $days_ago DAY),
    'PROCESO_$(($RANDOM % 10))',
    $(($RANDOM % 2)),
    'Test description for row $((i + j))',
    $(($RANDOM % 1000)),
    $(($RANDOM % 1000)),
    '$(python3 -c "import json; import random; data={'id': $((i+j)), 'data': ['x' * $json_size]}; print(json.dumps(data))")' ,
    'USER_$(($RANDOM % 5))',
    NOW(),
    '$(python3 -c "import json; import random; output={'result': 'success', 'data': ['y' * $json_size]}; print(json.dumps(output))")'
)
EOF
        done

        echo ";" >> /tmp/insert_batch.sql

        # Execute batch insert
        mysql -u$DB_USER -p$DB_PASS < /tmp/insert_batch.sql 2>/dev/null

        # Progress indicator
        echo -ne "\rInserted: $((i + current_chunk)) / $rows rows"
    done

    echo -e "\n${GREEN}✓ Test data generated successfully${NC}"
    rm -f /tmp/insert_batch.sql
}

# Function to run benchmark with specific strategy
run_benchmark() {
    local strategy=$1
    local batch_size=$2
    local description=$3

    echo -e "\n${MAGENTA}▸ Testing: $description${NC}"
    echo "  Strategy: $strategy | Batch Size: $batch_size"

    # Run purger and capture metrics
    START_TIME=$(date +%s%N)

    DATABASE_URL="mysql://$DB_USER:$DB_PASS@$DB_HOST/$DB_NAME" \
    timeout 300 $BINARY_PATH \
        --strategy $strategy \
        --batch-size $batch_size \
        --retention-days 90 \
        --sleep-ms 50 \
        --dry-run=false \
        2>&1 | tee /tmp/benchmark_output.txt

    END_TIME=$(date +%s%N)
    DURATION=$((($END_TIME - $START_TIME) / 1000000))

    # Extract metrics from output
    ROWS_DELETED=$(grep -oP 'Rows deleted:\s+\K\d+' /tmp/benchmark_output.txt | tail -1 || echo 0)

    if [ "$ROWS_DELETED" -gt 0 ]; then
        RATE=$((ROWS_DELETED * 1000 / DURATION))
        echo -e "  ${GREEN}Results:${NC}"
        echo "    Duration: ${DURATION}ms"
        echo "    Rows deleted: $ROWS_DELETED"
        echo "    Rate: $RATE rows/second"
    else
        echo -e "  ${RED}No rows deleted or error occurred${NC}"
    fi
}

# Function to benchmark different configurations
benchmark_suite() {
    local total_rows=$1

    echo -e "\n${BLUE}Starting Benchmark Suite${NC}"
    echo "═══════════════════════════════════════════════════════"

    # Test different strategies
    declare -a strategies=("id" "fecha" "adaptive")
    declare -a batch_sizes=(100 500 1000 2000)

    for strategy in "${strategies[@]}"; do
        echo -e "\n${YELLOW}Testing Strategy: $strategy${NC}"
        echo "───────────────────────────────────────────"

        for batch_size in "${batch_sizes[@]}"; do
            # Reset test data for each run
            generate_test_data $total_rows > /dev/null 2>&1

            run_benchmark $strategy $batch_size "$strategy strategy with batch size $batch_size"

            # Cool down between tests
            sleep 2
        done
    done
}

# Function to run stress test
stress_test() {
    local concurrent_purgers=$1

    echo -e "\n${YELLOW}Running Stress Test with $concurrent_purgers concurrent purgers${NC}"
    echo "───────────────────────────────────────────"

    # Generate large dataset
    generate_test_data 100000 > /dev/null 2>&1

    # Run multiple purgers concurrently
    for ((i=1; i<=$concurrent_purgers; i++)); do
        (
            DATABASE_URL="mysql://$DB_USER:$DB_PASS@$DB_HOST/$DB_NAME" \
            $BINARY_PATH \
                --strategy adaptive \
                --batch-size 500 \
                --retention-days 90 \
                --sleep-ms 100 \
                --user "USER_$((i % 5))" \
                > /tmp/purger_$i.log 2>&1
        ) &
    done

    # Wait for all to complete
    wait

    echo -e "${GREEN}✓ Stress test completed${NC}"

    # Show results
    for ((i=1; i<=$concurrent_purgers; i++)); do
        ROWS=$(grep -oP 'Rows deleted:\s+\K\d+' /tmp/purger_$i.log | tail -1 || echo 0)
        echo "  Purger $i deleted: $ROWS rows"
    done
}

# Function to benchmark memory usage
benchmark_memory() {
    echo -e "\n${YELLOW}Benchmarking Memory Usage${NC}"
    echo "───────────────────────────────────────────"

    generate_test_data 50000 > /dev/null 2>&1

    # Use /usr/bin/time for detailed memory stats
    /usr/bin/time -v \
        DATABASE_URL="mysql://$DB_USER:$DB_PASS@$DB_HOST/$DB_NAME" \
        $BINARY_PATH \
            --strategy adaptive \
            --batch-size 1000 \
            --retention-days 90 \
            2>&1 | grep -E "(Maximum resident|User time|System time|Elapsed)"
}

# Function to compare with other implementations
compare_implementations() {
    echo -e "\n${YELLOW}Comparing Implementations${NC}"
    echo "───────────────────────────────────────────"

    generate_test_data 10000 > /dev/null 2>&1

    # Rust implementation
    echo -e "\n${BLUE}Rust Implementation:${NC}"
    time DATABASE_URL="mysql://$DB_USER:$DB_PASS@$DB_HOST/$DB_NAME" \
        $BINARY_PATH \
            --strategy adaptive \
            --batch-size 500 \
            --retention-days 90

    # MySQL native DELETE (for comparison)
    echo -e "\n${BLUE}MySQL Native DELETE:${NC}"
    time mysql_exec "USE $DB_NAME; DELETE FROM log_sdk WHERE fecha < DATE_SUB(NOW(), INTERVAL 90 DAY);"

    # If you have Go implementation
    if [ -f "./go-purger" ]; then
        echo -e "\n${BLUE}Go Implementation:${NC}"
        time DATABASE_URL="mysql://$DB_USER:$DB_PASS@$DB_HOST/$DB_NAME" ./go-purger
    fi
}

# Main menu
show_menu() {
    echo -e "\n${BLUE}Benchmark Options:${NC}"
    echo "1) Quick benchmark (1,000 rows)"
    echo "2) Standard benchmark (10,000 rows)"
    echo "3) Large benchmark (100,000 rows)"
    echo "4) Stress test (concurrent purgers)"
    echo "5) Memory usage benchmark"
    echo "6) Compare implementations"
    echo "7) Full suite (all tests)"
    echo "8) Exit"
    echo -n "Select option: "
}

# Build the binary first
echo -e "${YELLOW}Building release binary...${NC}"
cargo build --release

# Main loop
while true; do
    show_menu
    read -r choice

    case $choice in
        1)
            generate_test_data 1000
            benchmark_suite 1000
            ;;
        2)
            generate_test_data 10000
            benchmark_suite 10000
            ;;
        3)
            generate_test_data 100000
            benchmark_suite 100000
            ;;
        4)
            echo -n "Number of concurrent purgers (1-10): "
            read -r num_purgers
            stress_test $num_purgers
            ;;
        5)
            benchmark_memory
            ;;
        6)
            compare_implementations
            ;;
        7)
            echo -e "${MAGENTA}Running Full Benchmark Suite${NC}"
            generate_test_data 10000
            benchmark_suite 10000
            stress_test 3
            benchmark_memory
            compare_implementations
            ;;
        8)
            echo -e "${GREEN}Exiting benchmark tool${NC}"
            exit 0
            ;;
        *)
            echo -e "${RED}Invalid option${NC}"
            ;;
    esac
done