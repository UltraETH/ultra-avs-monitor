#!/bin/bash

# Script to run various SQL queries against the bid_traces SQLite database.

# Configuration
DEFAULT_DB_PATH="./data/bids_all_outputs.db" # Default DB path, used by run_monitor_all_outputs.sh
DB_PATH="${1:-$DEFAULT_DB_PATH}" # Use first argument as DB path, or default

# Check if sqlite3 CLI is installed
if ! command -v sqlite3 &> /dev/null
then
    echo "sqlite3 command could not be found. Please install it to run this script."
    exit 1
fi

# Check if database file exists
if [ ! -f "$DB_PATH" ]; then
    echo "Database file not found at: $DB_PATH"
    echo "Please ensure the monitor service (e.g., run_monitor_all_outputs.sh or manage_monitor.sh start --mode all) has run and created the database."
    exit 1
fi

echo "Querying database: $DB_PATH"
echo "========================================="

run_query() {
    local title="$1"
    local query="$2"
    echo ""
    echo "--- $title ---"
    sqlite3 "$DB_PATH" "$query"
    echo "========================================="
}

# Query 1: Count total number of bids recorded
run_query "Total Bids Recorded" \
"SELECT COUNT(*) FROM bid_traces;"

# Query 2: Get the 5 most recent bids
run_query "5 Most Recent Bids (by received_at)" \
"SELECT block_number, value, builder_pubkey, received_at FROM bid_traces ORDER BY received_at DESC LIMIT 5;"

# Query 3: Get the 5 highest value bids
run_query "5 Highest Value Bids" \
"SELECT block_number, value, builder_pubkey, slot FROM bid_traces ORDER BY CAST(value AS INTEGER) DESC LIMIT 5;"

# Query 4: Count bids per builder_pubkey (Top 5 builders)
run_query "Bid Count per Builder (Top 5)" \
"SELECT builder_pubkey, COUNT(*) as bid_count FROM bid_traces GROUP BY builder_pubkey ORDER BY bid_count DESC LIMIT 5;"

# Query 5: Average bid value per builder_pubkey (Top 5 by average value)
# Note: SQLite's AVG on TEXT fields might not work as expected if values are very large.
# CAST to INTEGER is important.
run_query "Average Bid Value per Builder (Top 5 by Avg Value)" \
"SELECT builder_pubkey, AVG(CAST(value AS INTEGER)) as avg_value FROM bid_traces GROUP BY builder_pubkey ORDER BY avg_value DESC LIMIT 5;"

# Query 6: Bids for a specific block number (e.g., replace '123456' with a known block number)
# This is an example, you might want to pass the block number as an argument to the script in a more advanced version.
run_query "Bids for a Specific Block (Example: Block '123456')" \
"SELECT slot, value, builder_pubkey FROM bid_traces WHERE block_number = '123456';"

# Query 7: Number of unique block numbers with bids
run_query "Number of Unique Blocks with Bids" \
"SELECT COUNT(DISTINCT block_number) FROM bid_traces;"

# Query 8: Bids received within the last hour
# SQLite's datetime functions are powerful. 'now', '-1 hour'
run_query "Bids Received in the Last Hour" \
"SELECT block_number, value, builder_pubkey, received_at FROM bid_traces WHERE received_at >= datetime('now', '-1 hour') ORDER BY received_at DESC;"

# Query 9: Find bids with a value greater than a certain amount (e.g., 1000000000000000000 Wei = 1 ETH)
run_query "Bids with Value > 1 ETH (1000000000000000000 Wei)" \
"SELECT block_number, value, builder_pubkey FROM bid_traces WHERE CAST(value AS INTEGER) > 1000000000000000000 ORDER BY CAST(value AS INTEGER) DESC LIMIT 5;"

# Query 10: List all unique builder public keys
run_query "List All Unique Builder Public Keys" \
"SELECT DISTINCT builder_pubkey FROM bid_traces ORDER BY builder_pubkey LIMIT 10;" # Limit to 10 for brevity

echo ""
echo "Finished running queries."
