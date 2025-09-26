VS MySQL Purger

Purpose
- High-performance, safe, automated purging of the goals_logs.log_sdk table without blocking production. Implemented in Rust with adaptive strategies, health checks, and full observability.

Start Here
- Read docs/guidelines.md for the complete engineering guidelines and patterns, including:
  - Table schema hotspots (MEDIUMTEXT), risks, and constraints
  - Deletion strategies (ID, fecha, hybrid) and batch sizing
  - Health monitoring, error handling, and metrics/alerts
  - Operational runbook and maintenance
  - Optional partition migration (future optimization; not required)

Key Files
- src/ — purger implementation
- scripts/ — deploy and benchmark helpers
- .claude/context.txt — detailed background and design notes
- .claude/column_analysis.png — schema column analysis reference

Notes
- Partitioning is optional. The Rust purger works on the existing table as-is.
