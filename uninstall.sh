#!/bin/bash

# MediaRise Robot Console - Uninstallation Script

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
INSTALL_DIR="/opt/mediarise-robot-console"
SERVICE_USER="mediarise"
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
}

# Stop and remove service
remove_service() {
    echo -e "${YELLOW}Stopping and removing service...${NC}"
    
    if [[ "$OS" == "linux" ]]; then
        # Stop service
        if systemctl is-active --quiet "$PROJECT_NAME.service" 2>/dev/null; then
            systemctl stop "$PROJECT_NAME.service"
            echo -e "${GREEN}✓${NC} Service stopped"
        fi
        
        # Disable service
        if systemctl is-enabled --quiet "$PROJECT_NAME.service" 2>/dev/null; then
            systemctl disable "$PROJECT_NAME.service"
            echo -e "${GREEN}✓${NC} Service disabled"
        fi
        
        # Remove service file
        if [[ -f "/etc/systemd/system/$PROJECT_NAME.service" ]]; then
            rm "/etc/systemd/system/$PROJECT_NAME.service"
            systemctl daemon-reload
            echo -e "${GREEN}✓${NC} Service file removed"
        fi
        
    elif [[ "$OS" == "macos" ]]; then
        # Stop service
        if launchctl list | grep -q "com.mediarise.robot-console"; then
            launchctl unload "/Library/LaunchDaemons/com.mediarise.robot-console.plist" 2>/dev/null || true
            echo -e "${GREEN}✓${NC} Service stopped"
        fi
        
        # Remove plist file
        if [[ -f "/Library/LaunchDaemons/com.mediarise.robot-console.plist" ]]; then
            rm "/Library/LaunchDaemons/com.mediarise.robot-console.plist"
            echo -e "${GREEN}✓${NC} LaunchDaemon plist removed"
        fi
    fi
}

# Remove installation directory
remove_files() {
    echo -e "${YELLOW}Removing installation files...${NC}"
    
    if [[ -d "$INSTALL_DIR" ]]; then
        read -p "Remove installation directory $INSTALL_DIR? [y/N] " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            rm -rf "$INSTALL_DIR"
            echo -e "${GREEN}✓${NC} Installation directory removed"
        else
            echo -e "${YELLOW}⚠${NC} Installation directory kept"
        fi
    else
        echo -e "${YELLOW}⚠${NC} Installation directory not found"
    fi
}

# Remove service user (Linux only)
remove_user() {
    if [[ "$OS" == "linux" ]]; then
        if id "$SERVICE_USER" &>/dev/null; then
            read -p "Remove service user $SERVICE_USER? [y/N] " -n 1 -r
            echo
            if [[ $REPLY =~ ^[Yy]$ ]]; then
                userdel "$SERVICE_USER" 2>/dev/null || true
                echo -e "${GREEN}✓${NC} Service user removed"
            else
                echo -e "${YELLOW}⚠${NC} Service user kept"
            fi
        fi
    fi
}

# Main uninstallation
main() {
    echo -e "${RED}========================================${NC}"
    echo -e "${RED}MediaRise Robot Console Uninstallation${NC}"
    echo -e "${RED}========================================${NC}"
    echo ""
    
    detect_os
    check_root
    
    echo -e "${YELLOW}⚠${NC} This will remove the service and optionally the installation files."
    echo ""
    read -p "Continue? [y/N] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${YELLOW}Uninstallation cancelled${NC}"
        exit 0
    fi
    
    remove_service
    remove_files
    remove_user
    
    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}Uninstallation completed!${NC}"
    echo -e "${GREEN}========================================${NC}"
}

# Run main
main


