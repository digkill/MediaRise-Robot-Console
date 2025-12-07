//! MCP сервер

use anyhow::Context;
use serde_json::Value;
use tracing::{error, info, warn};

use crate::mcp::tools::{execute_tool, ToolRegistry};

/// JSON-RPC 2.0 Request
#[derive(Debug, serde::Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

/// JSON-RPC 2.0 Response
#[derive(Debug, serde::Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, serde::Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

pub struct McpServer {
    tool_registry: ToolRegistry,
}

impl McpServer {
    pub fn new() -> Self {
        Self {
            tool_registry: ToolRegistry::new(),
        }
    }

    pub async fn handle_request(
        &self,
        request: Value,
        services: Option<&crate::services::Services>,
    ) -> anyhow::Result<Value> {
        info!("MCP request: {}", request);

        let req: JsonRpcRequest =
            serde_json::from_value(request).context("Failed to parse JSON-RPC request")?;

        if req.jsonrpc != "2.0" {
            return Ok(self.error_response(
                req.id,
                -32600,
                "Invalid Request".to_string(),
                Some(serde_json::json!({"message": "jsonrpc must be '2.0'"})),
            ));
        }

        let result = match req.method.as_str() {
            "tools/list" => self.handle_tools_list().await,
            "tools/call" => {
                let tool_name = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("name"))
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing tool name"))?;

                let arguments = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("arguments"))
                    .cloned()
                    .unwrap_or(serde_json::json!({}));

                self.handle_tools_call(tool_name, arguments, services).await
            }
            "ping" => Ok(serde_json::json!({"pong": true})),
            _ => Err(anyhow::anyhow!("Unknown method: {}", req.method)),
        };

        match result {
            Ok(result_value) => Ok(self.success_response(req.id, result_value)),
            Err(e) => {
                error!("MCP request error: {}", e);
                Ok(self.error_response(
                    req.id,
                    -32603,
                    "Internal error".to_string(),
                    Some(serde_json::json!({"message": e.to_string()})),
                ))
            }
        }
    }

    async fn handle_tools_list(&self) -> anyhow::Result<Value> {
        let tools: Vec<Value> = self
            .tool_registry
            .list_tools()
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "tools": tools
        }))
    }

    async fn handle_tools_call(
        &self,
        tool_name: &str,
        arguments: Value,
        services: Option<&crate::services::Services>,
    ) -> anyhow::Result<Value> {
        let result = if let Some(services) = services {
            execute_tool(tool_name, arguments, services).await?
        } else {
            // Если services не предоставлены, возвращаем ошибку
            return Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": "Services not available"
                }],
                "isError": true,
            }));
        };

        Ok(serde_json::json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string(&result.result)?
            }],
            "isError": !result.success,
        }))
    }

    fn success_response(&self, id: Option<Value>, result: Value) -> Value {
        serde_json::to_value(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        })
        .unwrap_or(serde_json::json!({}))
    }

    fn error_response(
        &self,
        id: Option<Value>,
        code: i32,
        message: String,
        data: Option<Value>,
    ) -> Value {
        serde_json::to_value(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data,
            }),
        })
        .unwrap_or(serde_json::json!({}))
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}
