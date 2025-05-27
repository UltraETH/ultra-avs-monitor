#!/bin/bash

# Script to run the AVS Boost Monitor with both file and SQLite outputs enabled.

# Configuration
SERVICE_NAME="avs_boost_monitor_all"
PID_DIR="/tmp" # Or a more persistent location like /var/run/avs_boost_monitor
PID_FILE="${PID_DIR}/${SERVICE_NAME}.pid"
LOG_DIR="logs" # Relative to the project root
LOG_FILE="${LOG_DIR}/${SERVICE_NAME}.log"
ERROR_LOG_FILE="${LOG_DIR}/${SERVICE_NAME}_error.log"
APP_BINARY_PATH="./target/release/avs_boost_monitor" # Assuming release build
SQLITE_DB_PATH_ALL="./data/bids_all_outputs.db" # Specific DB for this mode
FILE_OUTPUT_PATH_ALL="./data/bids_all_outputs.jsonl" # Specific file for this mode

# Ensure log directory exists
mkdir -p "$LOG_DIR"

# Ensure PID directory exists
mkdir -p "$PID_DIR"

# Ensure data directory for outputs exists
mkdir -p "$(dirname "$SQLITE_DB_PATH_ALL")"
mkdir -p "$(dirname "$FILE_OUTPUT_PATH_ALL")"


# Function to check if the service is already running
is_running() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if ps -p "$PID" > /dev/null; then
            return 0 # Running
        else
            echo "Warning: Stale PID file found: $PID_FILE for PID $PID. Removing."
            rm -f "$PID_FILE"
            return 1 # Not running
        fi
    fi
    return 1 # Not running
}

start_service() {
    if is_running; then
        echo "$SERVICE_NAME is already running (PID: $(cat "$PID_FILE"))."
        exit 1
    fi

    echo "Starting $SERVICE_NAME..."

    if [ ! -f "$APP_BINARY_PATH" ]; then
        echo "Application binary not found at $APP_BINARY_PATH. Building..."
        cargo build --release
        if [ $? -ne 0 ]; then
            echo "Build failed. Exiting."
            exit 1
        fi
    fi

    export FILE_OUTPUT_ENABLED="true"
    export FILE_OUTPUT_PATH="$FILE_OUTPUT_PATH_ALL"
    export SQLITE_OUTPUT_ENABLED="true"
    export SQLITE_DATABASE_PATH="$SQLITE_DB_PATH_ALL"
    # Add other necessary env vars like ETHEREUM_RPC_URL, RELAY_URLS if not using a config file

    nohup "$APP_BINARY_PATH" \
        >> "$LOG_FILE" 2>> "$ERROR_LOG_FILE" &

    PID=$!
    echo "$PID" > "$PID_FILE"

    sleep 1

    if is_running; then
        echo "$SERVICE_NAME started successfully (PID: $PID)."
        echo "Logging to $LOG_FILE and $ERROR_LOG_FILE"
        echo "File output at $FILE_OUTPUT_PATH_ALL"
        echo "SQLite database at $SQLITE_DB_PATH_ALL"
    else
        echo "Failed to start $SERVICE_NAME. Check $LOG_FILE and $ERROR_LOG_FILE for details."
        rm -f "$PID_FILE"
        exit 1
    fi
}

# Main execution
start_service
