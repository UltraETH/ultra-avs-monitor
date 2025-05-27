#!/bin/bash

# Unified script to manage the AVS Boost Monitor service.

# Base Configuration (can be overridden by mode-specific settings)
DEFAULT_SERVICE_NAME="avs_boost_monitor_all" # Default mode if not specified
PID_DIR="/tmp"
LOG_DIR_BASE="logs"
APP_BINARY_PATH="./target/release/avs_boost_monitor"
CONFIG_FILE_PATH="./config/default.json" # Optional: Path to a JSON config file

# --- Mode Specific Configurations ---
# These will be set based on the --mode argument
SERVICE_NAME=""
PID_FILE=""
LOG_FILE=""
ERROR_LOG_FILE=""
ENV_VARS="" # String to hold environment variables for the specific mode

setup_mode_config() {
    local mode="$1"
    case "$mode" in
        file)
            SERVICE_NAME="avs_boost_monitor_file"
            ENV_VARS="export FILE_OUTPUT_ENABLED=true; export SQLITE_OUTPUT_ENABLED=false;"
            ;;
        sqlite)
            SERVICE_NAME="avs_boost_monitor_sqlite"
            ENV_VARS="export FILE_OUTPUT_ENABLED=false; export SQLITE_OUTPUT_ENABLED=true; export SQLITE_DATABASE_PATH='./data/bids_manage_sqlite.db';"
            # Ensure data directory for SQLite DB exists
            mkdir -p "$(dirname './data/bids_manage_sqlite.db')"
            ;;
        all|*) # Default to 'all' if mode is empty or unrecognized
            SERVICE_NAME="avs_boost_monitor_all"
            ENV_VARS="export FILE_OUTPUT_ENABLED=true; export FILE_OUTPUT_PATH='./data/bids_manage_all.jsonl'; export SQLITE_OUTPUT_ENABLED=true; export SQLITE_DATABASE_PATH='./data/bids_manage_all.db';"
            mkdir -p "$(dirname './data/bids_manage_all.jsonl')"
            mkdir -p "$(dirname './data/bids_manage_all.db')"
            ;;
    esac

    PID_FILE="${PID_DIR}/${SERVICE_NAME}.pid"
    LOG_FILE="${LOG_DIR_BASE}/${SERVICE_NAME}.log"
    ERROR_LOG_FILE="${LOG_DIR_BASE}/${SERVICE_NAME}_error.log"
    mkdir -p "$LOG_DIR_BASE"
    mkdir -p "$PID_DIR"
}


is_running() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if ps -p "$PID" > /dev/null; then
            return 0
        else
            echo "Warning: Stale PID file found: $PID_FILE for PID $PID. Removing."
            rm -f "$PID_FILE"
            return 1
        fi
    fi
    return 1
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

    # Construct command
    CMD_ARGS=""
    if [ -f "$CONFIG_FILE_PATH" ]; then
         CMD_ARGS="--config $CONFIG_FILE_PATH"
    fi
    # Add other CLI args if needed, e.g., --port, --metrics-port

    # Start the service in the background
    # The eval is used to correctly interpret the ENV_VARS string
    eval "$ENV_VARS"
    nohup "$APP_BINARY_PATH" $CMD_ARGS >> "$LOG_FILE" 2>> "$ERROR_LOG_FILE" &

    PID=$!
    echo "$PID" > "$PID_FILE"

    sleep 1

    if is_running; then
        echo "$SERVICE_NAME started successfully (PID: $PID)."
        echo "Mode: $CURRENT_MODE"
        echo "Logging to $LOG_FILE and $ERROR_LOG_FILE"
        if [[ "$ENV_VARS" == *"FILE_OUTPUT_ENABLED=true"* ]]; then
            FILE_PATH_TO_LOG=$(echo "$ENV_VARS" | grep -o "FILE_OUTPUT_PATH='[^']*'" | cut -d"'" -f2)
            if [ -z "$FILE_PATH_TO_LOG" ]; then FILE_PATH_TO_LOG="./data/bids.jsonl"; fi # Default if not in ENV_VARS
            echo "File output at $FILE_PATH_TO_LOG"
        fi
        if [[ "$ENV_VARS" == *"SQLITE_OUTPUT_ENABLED=true"* ]]; then
            SQLITE_PATH_TO_LOG=$(echo "$ENV_VARS" | grep -o "SQLITE_DATABASE_PATH='[^']*'" | cut -d"'" -f2)
             if [ -z "$SQLITE_PATH_TO_LOG" ]; then SQLITE_PATH_TO_LOG="./data/bids.db"; fi # Default if not in ENV_VARS
            echo "SQLite database at $SQLITE_PATH_TO_LOG"
        fi
    else
        echo "Failed to start $SERVICE_NAME. Check $LOG_FILE and $ERROR_LOG_FILE for details."
        rm -f "$PID_FILE"
        exit 1
    fi
}

stop_service() {
    if ! is_running; then
        echo "$SERVICE_NAME is not running."
        exit 0
    fi

    PID=$(cat "$PID_FILE")
    echo "Stopping $SERVICE_NAME (PID: $PID)..."

    # Try to gracefully terminate
    kill "$PID"
    sleep 2 # Wait for graceful shutdown

    if ps -p "$PID" > /dev/null; then
        echo "$SERVICE_NAME (PID: $PID) did not stop gracefully. Sending SIGKILL..."
        kill -9 "$PID"
        sleep 1
    fi

    if ps -p "$PID" > /dev/null; then
        echo "Failed to stop $SERVICE_NAME (PID: $PID) even with SIGKILL."
        exit 1
    else
        echo "$SERVICE_NAME stopped."
        rm -f "$PID_FILE"
    fi
}

status_service() {
    if is_running; then
        echo "$SERVICE_NAME is running (PID: $(cat "$PID_FILE")). Mode: $CURRENT_MODE"
    else
        echo "$SERVICE_NAME is not running. Mode: $CURRENT_MODE"
    fi
}

# Main script logic
ACTION="$1"
MODE_ARG="$2" # Optional mode argument, e.g., --mode file

CURRENT_MODE="all" # Default
if [[ "$MODE_ARG" == "--mode" ]]; then
    if [ -n "$3" ]; then
        CURRENT_MODE="$3"
    else
        echo "Error: --mode option requires a mode (file, sqlite, all)."
        exit 1
    fi
fi
setup_mode_config "$CURRENT_MODE"


case "$ACTION" in
    start)
        start_service
        ;;
    stop)
        stop_service
        ;;
    restart)
        stop_service
        sleep 1
        start_service
        ;;
    status)
        status_service
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|status} [--mode {file|sqlite|all}]"
        exit 1
        ;;
esac

exit 0
