-- Initial database schema

-- Devices table
CREATE TABLE IF NOT EXISTS devices (
    device_id VARCHAR(64) PRIMARY KEY,
    client_id VARCHAR(64) NOT NULL,
    serial_number VARCHAR(64),
    firmware_version VARCHAR(32) NOT NULL,
    activated BOOLEAN DEFAULT FALSE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Sessions table
CREATE TABLE IF NOT EXISTS sessions (
    id CHAR(36) PRIMARY KEY,
    device_id VARCHAR(64) NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_activity TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (device_id) REFERENCES devices(device_id)
);

-- Firmware versions table
CREATE TABLE IF NOT EXISTS firmware_versions (
    version VARCHAR(32) PRIMARY KEY,
    url TEXT NOT NULL,
    force_update BOOLEAN DEFAULT FALSE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Assets versions table
CREATE TABLE IF NOT EXISTS assets_versions (
    version VARCHAR(32) PRIMARY KEY,
    url TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Uploads table
CREATE TABLE IF NOT EXISTS uploads (
    id CHAR(36) PRIMARY KEY,
    device_id VARCHAR(64) NOT NULL,
    file_path TEXT NOT NULL,
    file_type VARCHAR(64) NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (device_id) REFERENCES devices(device_id)
);

-- Create indexes
CREATE INDEX idx_devices_client_id ON devices(client_id);
CREATE INDEX idx_devices_serial_number ON devices(serial_number);
CREATE INDEX idx_sessions_device_id ON sessions(device_id);
