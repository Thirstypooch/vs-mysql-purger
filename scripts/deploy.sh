#!/bin/bash
set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}Deploying Log SDK Purger...${NC}"

# Build release binary
echo "Building release binary..."
cargo build --release

# Run tests
echo "Running tests..."
cargo test --release

# Create systemd service
sudo tee /etc/systemd/system/log-sdk-purger.service > /dev/null <<EOF
[Unit]
Description=Log SDK Purger - High Performance
After=network.target mysql.service
Wants=mysql.service

[Service]
Type=simple
User=mysql
Group=mysql

# Environment
Environment="DATABASE_URL=mysql://goals_plazatodo:Swut_pJ!jcn/2*-OC7z@15.235.11.55:3308/goals_logs"
Environment="RUST_LOG=info"

# Execution
ExecStart=/usr/local/bin/log-sdk-purger \
    --strategy adaptive \
    --batch-size 500 \
    --sleep-ms 200 \
    --retention-days 90 \
    --max-connections 30 \
    --metrics-port 9090 \
    --progress

# Restart policy
Restart=on-failure
RestartSec=30s
MaxRestartSec=300s

# Resource limits
LimitNOFILE=65535
MemoryMax=512M
CPUQuota=50%
IOWeight=50

# Security
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/log/log-sdk-purger

[Install]
WantedBy=multi-user.target
EOF

# Copy binary
sudo cp target/release/log-sdk-purger /usr/local/bin/
sudo chmod +x /usr/local/bin/log-sdk-purger

# Create log directory
sudo mkdir -p /var/log/log-sdk-purger
sudo chown mysql:mysql /var/log/log-sdk-purger

# Reload systemd
sudo systemctl daemon-reload
sudo systemctl enable log-sdk-purger

echo -e "${GREEN}Deployment complete!${NC}"
echo "Start with: sudo systemctl start log-sdk-purger"
echo "View logs: journalctl -u log-sdk-purger -f"
echo "Metrics available at: http://localhost:9090/metrics"