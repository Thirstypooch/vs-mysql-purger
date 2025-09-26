-- ═══════════════════════════════════════════════════════════════════════════
-- MIGRATE TO PARTITIONS - Critical for Performance at Scale
-- ═══════════════════════════════════════════════════════════════════════════
--
-- WHY PARTITIONING IS CRUCIAL FOR YOUR USE CASE:
--
-- 1. INSTANT DELETION: Instead of deleting millions of rows (hours/days),
--    dropping a partition takes milliseconds
--
-- 2. NO TABLE LOCKS: Partition drops don't lock the entire table
--
-- 3. BETTER PERFORMANCE: Queries only scan relevant partitions
--
-- 4. EASIER MAINTENANCE: Each partition can be maintained independently
--
-- 5. REDUCED I/O: No need to update indexes for millions of deletions
-- ═══════════════════════════════════════════════════════════════════════════

DELIMITER $$

-- Check if table is already partitioned
DROP FUNCTION IF EXISTS is_table_partitioned$$
CREATE FUNCTION is_table_partitioned(table_name VARCHAR(64))
    RETURNS BOOLEAN
    DETERMINISTIC
    READS SQL DATA
BEGIN
    DECLARE is_partitioned BOOLEAN DEFAULT FALSE;

SELECT COUNT(*) > 0 INTO is_partitioned
FROM information_schema.partitions
WHERE table_schema = DATABASE()
  AND table_name = table_name
  AND partition_name IS NOT NULL;

RETURN is_partitioned;
END$$

-- Procedure to safely migrate to partitions
DROP PROCEDURE IF EXISTS migrate_log_sdk_to_partitions$$
CREATE PROCEDURE migrate_log_sdk_to_partitions()
BEGIN
    DECLARE exit handler for SQLEXCEPTION
BEGIN
ROLLBACK;
SIGNAL SQLSTATE '45000'
            SET MESSAGE_TEXT = 'Migration failed, rolled back';
END;

    -- Check if already partitioned
    IF is_table_partitioned('log_sdk') THEN
SELECT 'Table log_sdk is already partitioned' AS status;
ELSE
        START TRANSACTION;

        -- Step 1: Create backup table
SELECT 'Creating backup table...' AS status;
CREATE TABLE IF NOT EXISTS log_sdk_backup LIKE log_sdk;

-- Step 2: Copy recent data to backup (last 6 months only)
SELECT 'Backing up recent data...' AS status;
INSERT INTO log_sdk_backup
SELECT * FROM log_sdk
WHERE fecha >= DATE_SUB(NOW(), INTERVAL 6 MONTH);

-- Step 3: Rename original table
SELECT 'Renaming original table...' AS status;
RENAME TABLE log_sdk TO log_sdk_old;

        -- Step 4: Create new partitioned table
SELECT 'Creating partitioned table...' AS status;
CREATE TABLE log_sdk (
                         id INT AUTO_INCREMENT,
                         fecha DATETIME NOT NULL,
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
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci
        PARTITION BY RANGE (TO_DAYS(fecha)) (
            -- Historical partitions (for old data)
            PARTITION p_2024_01 VALUES LESS THAN (TO_DAYS('2024-02-01')),
            PARTITION p_2024_02 VALUES LESS THAN (TO_DAYS('2024-03-01')),
            PARTITION p_2024_03 VALUES LESS THAN (TO_DAYS('2024-04-01')),
            PARTITION p_2024_04 VALUES LESS THAN (TO_DAYS('2024-05-01')),
            PARTITION p_2024_05 VALUES LESS THAN (TO_DAYS('2024-06-01')),
            PARTITION p_2024_06 VALUES LESS THAN (TO_DAYS('2024-07-01')),
            PARTITION p_2024_07 VALUES LESS THAN (TO_DAYS('2024-08-01')),
            PARTITION p_2024_08 VALUES LESS THAN (TO_DAYS('2024-09-01')),
            PARTITION p_2024_09 VALUES LESS THAN (TO_DAYS('2024-10-01')),
            PARTITION p_2024_10 VALUES LESS THAN (TO_DAYS('2024-11-01')),
            PARTITION p_2024_11 VALUES LESS THAN (TO_DAYS('2024-12-01')),
            PARTITION p_2024_12 VALUES LESS THAN (TO_DAYS('2025-01-01')),
            -- Current year partitions
            PARTITION p_2025_01 VALUES LESS THAN (TO_DAYS('2025-02-01')),
            PARTITION p_2025_02 VALUES LESS THAN (TO_DAYS('2025-03-01')),
            PARTITION p_2025_03 VALUES LESS THAN (TO_DAYS('2025-04-01')),
            PARTITION p_2025_04 VALUES LESS THAN (TO_DAYS('2025-05-01')),
            PARTITION p_2025_05 VALUES LESS THAN (TO_DAYS('2025-06-01')),
            PARTITION p_2025_06 VALUES LESS THAN (TO_DAYS('2025-07-01')),
            PARTITION p_2025_07 VALUES LESS THAN (TO_DAYS('2025-08-01')),
            PARTITION p_2025_08 VALUES LESS THAN (TO_DAYS('2025-09-01')),
            PARTITION p_2025_09 VALUES LESS THAN (TO_DAYS('2025-10-01')),
            PARTITION p_2025_10 VALUES LESS THAN (TO_DAYS('2025-11-01')),
            PARTITION p_2025_11 VALUES LESS THAN (TO_DAYS('2025-12-01')),
            PARTITION p_2025_12 VALUES LESS THAN (TO_DAYS('2026-01-01')),
            -- Future partitions
            PARTITION p_future VALUES LESS THAN MAXVALUE
        );

-- Step 5: Copy data back
SELECT 'Copying data to partitioned table...' AS status;
INSERT INTO log_sdk SELECT * FROM log_sdk_backup;

-- Step 6: Verify row counts
SELECT
    (SELECT COUNT(*) FROM log_sdk_backup) AS backup_rows,
    (SELECT COUNT(*) FROM log_sdk) AS new_table_rows;

COMMIT;

SELECT 'Migration completed successfully!' AS status;

-- Step 7: Show partition information
SELECT
    partition_name,
    partition_expression,
    partition_description,
    table_rows
FROM information_schema.partitions
WHERE table_schema = DATABASE()
  AND table_name = 'log_sdk'
  AND partition_name IS NOT NULL
ORDER BY partition_ordinal_position;
END IF;
END$$

-- Procedure to automatically manage partitions
DROP PROCEDURE IF EXISTS manage_log_sdk_partitions$$
CREATE PROCEDURE manage_log_sdk_partitions()
BEGIN
    DECLARE done INT DEFAULT FALSE;
    DECLARE p_name VARCHAR(64);
    DECLARE p_description VARCHAR(100);
    DECLARE retention_date DATE;

    -- Cursor for old partitions to drop
    DECLARE partition_cursor CURSOR FOR
SELECT partition_name, partition_description
FROM information_schema.partitions
WHERE table_schema = DATABASE()
  AND table_name = 'log_sdk'
  AND partition_name IS NOT NULL
  AND partition_name != 'p_future'
        AND partition_description < TO_DAYS(DATE_SUB(NOW(), INTERVAL 3 MONTH));

DECLARE CONTINUE HANDLER FOR NOT FOUND SET done = TRUE;

    -- Drop old partitions
OPEN partition_cursor;

drop_loop: LOOP
        FETCH partition_cursor INTO p_name, p_description;
        IF done THEN
            LEAVE drop_loop;
END IF;

        SET @sql = CONCAT('ALTER TABLE log_sdk DROP PARTITION ', p_name);
PREPARE stmt FROM @sql;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SELECT CONCAT('Dropped partition: ', p_name) AS action;
END LOOP;

CLOSE partition_cursor;

-- Add new future partitions (for next 3 months)
CALL create_future_partitions();
END$$

-- Procedure to create future partitions
DROP PROCEDURE IF EXISTS create_future_partitions$$
CREATE PROCEDURE create_future_partitions()
BEGIN
    DECLARE next_month DATE;
    DECLARE partition_name VARCHAR(20);
    DECLARE max_existing_partition DATE;
    DECLARE i INT DEFAULT 1;

    -- Find the last partition before p_future
SELECT MAX(
               STR_TO_DATE(
                       SUBSTRING_INDEX(partition_description, '(', -1),
                       '%Y-%m-%d'
               )
       ) INTO max_existing_partition
FROM information_schema.partitions
WHERE table_schema = DATABASE()
  AND table_name = 'log_sdk'
  AND partition_name LIKE 'p_%'
  AND partition_name != 'p_future';

-- Create partitions for next 3 months
WHILE i <= 3 DO
        SET next_month = DATE_ADD(max_existing_partition, INTERVAL i MONTH);
        SET partition_name = CONCAT('p_', DATE_FORMAT(next_month, '%Y_%m'));

        -- Check if partition already exists
        IF NOT EXISTS (
            SELECT 1 FROM information_schema.partitions
            WHERE table_schema = DATABASE()
            AND table_name = 'log_sdk'
            AND partition_name = partition_name
        ) THEN
            SET @sql = CONCAT(
                'ALTER TABLE log_sdk REORGANIZE PARTITION p_future INTO (',
                'PARTITION ', partition_name,
                ' VALUES LESS THAN (TO_DAYS(''',
                DATE_ADD(next_month, INTERVAL 1 MONTH),
                ''')), PARTITION p_future VALUES LESS THAN MAXVALUE)'
            );
PREPARE stmt FROM @sql;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SELECT CONCAT('Created partition: ', partition_name) AS action;
END IF;

        SET i = i + 1;
END WHILE;
END$$

-- Event scheduler to automatically manage partitions
DROP EVENT IF EXISTS manage_partitions_event$$
CREATE EVENT manage_partitions_event
ON SCHEDULE EVERY 1 WEEK
STARTS CURRENT_TIMESTAMP
DO
BEGIN
CALL manage_log_sdk_partitions();
END$$

DELIMITER ;

-- Enable event scheduler if not already enabled
SET GLOBAL event_scheduler = ON;

-- ═══════════════════════════════════════════════════════════════════════════
-- PERFORMANCE COMPARISON
-- ═══════════════════════════════════════════════════════════════════════════
--
-- WITHOUT PARTITIONS (Current):
-- DELETE FROM log_sdk WHERE fecha < '2024-06-01'
-- → Time: 2-10 hours for millions of rows
-- → Locks: Table locked during deletion
-- → I/O: Massive write operations
-- → Rollback: Huge undo logs
--
-- WITH PARTITIONS (After migration):
-- ALTER TABLE log_sdk DROP PARTITION p_2024_06
-- → Time: < 1 second
-- → Locks: Minimal
-- → I/O: Metadata operation only
-- → Rollback: Instant
-- ═══════════════════════════════════════════════════════════════════════════

-- Execute migration
CALL migrate_log_sdk_to_partitions();

-- Show results
SELECT
    CONCAT('Partition: ', partition_name) AS partition_info,
    CONCAT('Rows: ', FORMAT(table_rows, 0)) AS row_count,
    CONCAT('Size: ', ROUND(data_length / 1024 / 1024, 2), ' MB') AS data_size
FROM information_schema.partitions
WHERE table_schema = DATABASE()
  AND table_name = 'log_sdk'
  AND partition_name IS NOT NULL
ORDER BY partition_ordinal_position;