#!/bin/bash

# MediaRise Robot Console - Installation Script
# This script installs the server as a system service (systemd on Linux, LaunchDaemon on macOS)

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
INSTALL_DIR="/opt/mediarise-robot-console"
BIN_DIR="$INSTALL_DIR/bin"
SERVICE_USER="mediarise"
SERVICE_GROUP="mediarise"
PROJECT_NAME="mediarise-robot-console"

# Detect OS
detect_os() {
    if [[ "$OSTYPE" == "linux-gnu"* ]]; then
        OS="linux"
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        OS="macos"
    else
        echo -e "${RED}❌ Unsupported OS: $OSTYPE${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓${NC} Detected OS: $OS"
}

# Check if running as root
check_root() {
    if [[ $EUID -ne 0 ]]; then
        echo -e "${RED}❌ This script must be run as root (use sudo)${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓${NC} Running as root"
}

# Check dependencies
check_dependencies() {
    echo -e "${YELLOW}Checking dependencies...${NC}"
    
    # Check Rust
    if ! command -v cargo &> /dev/null; then
        echo -e "${RED}❌ Rust/Cargo not found. Please install Rust: https://rustup.rs/${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓${NC} Rust/Cargo found: $(cargo --version)"
    
    # Check systemd (Linux only)
    if [[ "$OS" == "linux" ]]; then
        if ! command -v systemctl &> /dev/null; then
            echo -e "${RED}❌ systemctl not found. This script requires systemd.${NC}"
            exit 1
        fi
        echo -e "${GREEN}✓${NC} systemd found"
    fi
    
    # Check launchctl (macOS only)
    if [[ "$OS" == "macos" ]]; then
        if ! command -v launchctl &> /dev/null; then
            echo -e "${RED}❌ launchctl not found.${NC}"
            exit 1
        fi
        echo -e "${GREEN}✓${NC} launchctl found"
    fi
}

# Create service user (Linux only)
create_service_user() {
    if [[ "$OS" == "linux" ]]; then
        if id "$SERVICE_USER" &>/dev/null; then
            echo -e "${YELLOW}⚠${NC} User $SERVICE_USER already exists"
        else
            echo -e "${YELLOW}Creating service user...${NC}"
            useradd -r -s /bin/false -d "$INSTALL_DIR" "$SERVICE_USER"
            echo -e "${GREEN}✓${NC} Created user: $SERVICE_USER"
        fi
    fi
}

# Build the project
build_project() {
    echo -e "${YELLOW}Building project...${NC}"
    
    # Build release version
    cargo build --release
    
    if [[ ! -f "target/release/$PROJECT_NAME" ]]; then
        echo -e "${RED}❌ Build failed - binary not found${NC}"
        exit 1
    fi
    
    echo -e "${GREEN}✓${NC} Build successful"
}

# Create installation directories
create_directories() {
    echo -e "${YELLOW}Creating installation directories...${NC}"
    
    mkdir -p "$INSTALL_DIR"
    mkdir -p "$BIN_DIR"
    mkdir -p "$INSTALL_DIR/storage/firmware"
    mkdir -p "$INSTALL_DIR/storage/assets"
    mkdir -p "$INSTALL_DIR/storage/uploads"
    mkdir -p "$INSTALL_DIR/logs"
    
    # Copy .env.example if .env doesn't exist
    if [[ ! -f "$INSTALL_DIR/.env" ]] && [[ -f ".env.example" ]]; then
        cp ".env.example" "$INSTALL_DIR/.env"
        echo -e "${YELLOW}⚠${NC} Created .env from .env.example - please configure it!"
    fi
    
    echo -e "${GREEN}✓${NC} Directories created"
}

# Install files
install_files() {
    echo -e "${YELLOW}Installing files...${NC}"
    
    # Copy binary
    cp "target/release/$PROJECT_NAME" "$BIN_DIR/$PROJECT_NAME"
    chmod +x "$BIN_DIR/$PROJECT_NAME"
    echo -e "${GREEN}✓${NC} Binary installed"
    
    # Copy .env if it exists in current directory
    if [[ -f ".env" ]]; then
        cp ".env" "$INSTALL_DIR/.env"
        chmod 600 "$INSTALL_DIR/.env"
        echo -e "${GREEN}✓${NC} .env file copied"
    fi
    
    # Set ownership (Linux only)
    if [[ "$OS" == "linux" ]]; then
        chown -R "$SERVICE_USER:$SERVICE_GROUP" "$INSTALL_DIR"
        echo -e "${GREEN}✓${NC} Ownership set to $SERVICE_USER:$SERVICE_GROUP"
    fi
}

# Install systemd service (Linux)
install_systemd_service() {
    if [[ "$OS" != "linux" ]]; then
        return
    fi
    
    echo -e "${YELLOW}Installing systemd service...${NC}"
    
    # Copy service file
    cp "systemd/$PROJECT_NAME.service" "/etc/systemd/system/$PROJECT_NAME.service"
    
    # Reload systemd
    systemctl daemon-reload
    
    # Enable service
    systemctl enable "$PROJECT_NAME.service"
    
    echo -e "${GREEN}✓${NC} Systemd service installed and enabled"
    echo -e "${YELLOW}⚠${NC} Service is not started yet. Use 'systemctl start $PROJECT_NAME' to start it."
}

# Install LaunchDaemon (macOS)
install_launchdaemon() {
    if [[ "$OS" != "macos" ]]; then
        return
    fi
    
    echo -e "${YELLOW}Installing LaunchDaemon...${NC}"
    
    # Stop existing service if running
    if launchctl list | grep -q "com.mediarise.robot-console"; then
        echo -e "${YELLOW}Stopping existing service...${NC}"
        launchctl unload "/Library/LaunchDaemons/com.mediarise.robot-console.plist" 2>/dev/null || true
    fi
    
    # Copy plist file
    cp "macos/com.mediarise.robot-console.plist" "/Library/LaunchDaemons/com.mediarise.robot-console.plist"
    chmod 644 "/Library/LaunchDaemons/com.mediarise.robot-console.plist"
    
    # Load service
    launchctl load "/Library/LaunchDaemons/com.mediarise.robot-console.plist"
    
    echo -e "${GREEN}✓${NC} LaunchDaemon installed and loaded"
    echo -e "${YELLOW}⚠${NC} Service should start automatically. Check logs in $INSTALL_DIR/logs/"
}

# Main installation
main() {
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}MediaRise Robot Console Installation${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    
    detect_os
    check_root
    check_dependencies
    create_service_user
    build_project
    create_directories
    install_files
    
    if [[ "$OS" == "linux" ]]; then
        install_systemd_service
    elif [[ "$OS" == "macos" ]]; then
        install_launchdaemon
    fi
    
    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}Installation completed!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo -e "Installation directory: ${YELLOW}$INSTALL_DIR${NC}"
    echo -e "Binary: ${YELLOW}$BIN_DIR/$PROJECT_NAME${NC}"
    echo ""
    
    if [[ "$OS" == "linux" ]]; then
        echo -e "Service management:"
        echo -e "  Start:   ${YELLOW}systemctl start $PROJECT_NAME${NC}"
        echo -e "  Stop:    ${YELLOW}systemctl stop $PROJECT_NAME${NC}"
        echo -e "  Status:  ${YELLOW}systemctl status $PROJECT_NAME${NC}"
        echo -e "  Logs:    ${YELLOW}journalctl -u $PROJECT_NAME -f${NC}"
    elif [[ "$OS" == "macos" ]]; then
        echo -e "Service management:"
        echo -e "  Start:   ${YELLOW}sudo launchctl load /Library/LaunchDaemons/com.mediarise.robot-console.plist${NC}"
        echo -e "  Stop:    ${YELLOW}sudo launchctl unload /Library/LaunchDaemons/com.mediarise.robot-console.plist${NC}"
        echo -e "  Status:  ${YELLOW}launchctl list | grep com.mediarise.robot-console${NC}"
        echo -e "  Logs:    ${YELLOW}tail -f $INSTALL_DIR/logs/stdout.log${NC}"
    fi
    
    echo ""
    echo -e "${YELLOW}⚠${NC} Don't forget to configure $INSTALL_DIR/.env file!"
}

# Run main
main


