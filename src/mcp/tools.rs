//! MCP инструменты

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Регистр инструментов MCP
pub struct ToolRegistry {
    tools: Vec<Tool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut registry = Self { tools: Vec::new() };

        // Регистрируем стандартные инструменты
        registry.register_tool(Tool {
            name: "get_device_status".to_string(),
            description: "Получить статус устройства".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "device_id": {
                        "type": "string",
                        "description": "ID устройства"
                    }
                },
                "required": ["device_id"]
            }),
        });

        registry.register_tool(Tool {
            name: "send_command".to_string(),
            description: "Отправить команду устройству".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "device_id": {
                        "type": "string",
                        "description": "ID устройства"
                    },
                    "command": {
                        "type": "string",
                        "description": "Команда для выполнения"
                    }
                },
                "required": ["device_id", "command"]
            }),
        });

        registry.register_tool(Tool {
            name: "get_system_info".to_string(),
            description: "Получить системную информацию".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        });

        registry
    }

    pub fn register_tool(&mut self, tool: Tool) {
        self.tools.push(tool);
    }

    pub fn list_tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn find_tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.name == name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Результат выполнения инструмента
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub result: Value,
    pub error: Option<String>,
}

/// Выполняет инструмент
pub async fn execute_tool(
    tool_name: &str,
    arguments: Value,
    services: &crate::services::Services,
) -> Result<ToolResult> {
    match tool_name {
        "get_device_status" => {
            let device_id = arguments["device_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing device_id"))?;

            let device = services.device.get_device(device_id).await?;

            Ok(ToolResult {
                success: true,
                result: serde_json::json!({
                    "device": device.map(|d| serde_json::json!({
                        "device_id": d.device_id,
                        "client_id": d.client_id,
                        "firmware_version": d.firmware_version,
                        "activated": d.activated,
                        "last_seen": d.last_seen.to_rfc3339(),
                    })),
                }),
                error: None,
            })
        }
        "send_command" => {
            let device_id = arguments["device_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing device_id"))?;
            let command = arguments["command"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing command"))?;

            // TODO: Отправить команду через MQTT или WebSocket
            Ok(ToolResult {
                success: true,
                result: serde_json::json!({
                    "message": format!("Command '{}' sent to device {}", command, device_id),
                }),
                error: None,
            })
        }
        "get_system_info" => {
            Ok(ToolResult {
                success: true,
                result: serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "uptime": "N/A", // TODO: Calculate actual uptime
                }),
                error: None,
            })
        }
        _ => Ok(ToolResult {
            success: false,
            result: serde_json::json!({}),
            error: Some(format!("Unknown tool: {}", tool_name)),
        }),
    }
}
