use async_trait::async_trait;
use mcp_gateway_ir::{CompileBundle, Operation};
use mcp_gateway_proxy::{
    error_result, map_proxy_error, map_success, render, validate_and_dial, validate_schema,
    InjectedCredential, ProxyError, Resolver, SsrfPolicy, ToolResult, UpstreamResponse,
};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData as McpError,
    Implementation, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::RoleServer;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

#[cfg(test)]
use std::collections::VecDeque;

const PAGE_SIZE: usize = 50;
const SCHEMA_BUDGET: Duration = Duration::from_millis(50);

#[async_trait]
pub trait UpstreamExecutor: Send + Sync {
    async fn execute(
        &self,
        req: mcp_gateway_proxy::RenderedRequest,
        policy: SsrfPolicy,
        cred: Option<&InjectedCredential>,
    ) -> Result<UpstreamResponse, ProxyError>;
}

pub struct LiveExecutor {
    pub resolver: Arc<dyn Resolver>,
}

#[async_trait]
impl UpstreamExecutor for LiveExecutor {
    async fn execute(
        &self,
        req: mcp_gateway_proxy::RenderedRequest,
        policy: SsrfPolicy,
        cred: Option<&InjectedCredential>,
    ) -> Result<UpstreamResponse, ProxyError> {
        validate_and_dial(req, &policy, self.resolver.as_ref(), cred).await
    }
}

#[cfg(test)]
pub struct SequenceExecutor {
    responses: std::sync::Mutex<VecDeque<Result<UpstreamResponse, ProxyError>>>,
}

#[cfg(test)]
impl SequenceExecutor {
    pub fn from_responses(
        responses: impl IntoIterator<Item = Result<UpstreamResponse, ProxyError>>,
    ) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses.into_iter().collect()),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl UpstreamExecutor for SequenceExecutor {
    async fn execute(
        &self,
        _req: mcp_gateway_proxy::RenderedRequest,
        _policy: SsrfPolicy,
        _cred: Option<&InjectedCredential>,
    ) -> Result<UpstreamResponse, ProxyError> {
        self.responses
            .lock()
            .expect("exec")
            .pop_front()
            .unwrap_or_else(|| Err(ProxyError::Upstream("no mock response".into())))
    }
}

#[derive(Clone)]
pub struct LocalGateway {
    pub bundle: Arc<CompileBundle>,
    pub base_url: String,
    pub credential: Option<InjectedCredential>,
    pub ssrf: SsrfPolicy,
    pub enabled_tools: Vec<String>,
    pub disabled_tools: Vec<String>,
}

impl LocalGateway {
    pub fn operations(&self) -> impl Iterator<Item = &Operation> {
        self.bundle
            .api
            .operations
            .iter()
            .filter(|op| self.tool_allowed(&op.tool.name))
    }

    pub fn operation(&self, name: &str) -> Option<&Operation> {
        if !self.tool_allowed(name) {
            return None;
        }
        self.bundle
            .api
            .operations
            .iter()
            .find(|op| op.tool.name == name)
    }

    fn tool_allowed(&self, name: &str) -> bool {
        if !self.enabled_tools.is_empty() && !self.enabled_tools.iter().any(|n| n == name) {
            return false;
        }
        !self.disabled_tools.iter().any(|n| n == name)
    }
}

#[derive(Clone)]
pub struct GatewayHandler {
    pub gateway: Arc<LocalGateway>,
    pub executor: Arc<dyn UpstreamExecutor>,
}

impl GatewayHandler {
    pub fn new(gateway: Arc<LocalGateway>, executor: Arc<dyn UpstreamExecutor>) -> Self {
        Self { gateway, executor }
    }

    pub async fn execute_named(&self, name: &str, arguments: Value) -> ToolResult {
        let Some(op) = self.gateway.operation(name) else {
            return error_result("unknown_tool", "Unknown tool");
        };
        self.execute_operation(op, arguments).await
    }

    async fn execute_operation(&self, op: &Operation, arguments: Value) -> ToolResult {
        if let Err(msg) = tokio::time::timeout(SCHEMA_BUDGET, async {
            validate_schema(&op.tool.input_schema, &arguments)
        })
        .await
        .unwrap_or(Err("schema validation timed out".into()))
        {
            return error_result("invalid_arguments", &msg);
        }

        let rendered = match render(&self.gateway.base_url, &op.execution_plan, &arguments) {
            Ok(r) => r,
            Err(e) => return map_proxy_error(&e, None),
        };

        match self
            .executor
            .execute(
                rendered,
                self.gateway.ssrf.clone(),
                self.gateway.credential.as_ref(),
            )
            .await
        {
            Ok(upstream) => map_success(&upstream, mcp_gateway_proxy::ResponseKind::Json, None),
            Err(e) => {
                if !matches!(e, ProxyError::Ssrf(_)) {
                    warn!(error = %e, tool = %op.tool.name, "upstream call failed");
                }
                map_proxy_error(&e, None)
            }
        }
    }
}

impl ServerHandler for GatewayHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
            .with_server_info(Implementation::new(
                "mcp-gateway",
                env!("CARGO_PKG_VERSION"),
            ))
    }

    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [ProtocolVersion]> {
        std::borrow::Cow::Borrowed(&[ProtocolVersion::V_2026_07_28])
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let start = request
            .as_ref()
            .and_then(|p| p.cursor.as_deref())
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(0);
        let ops: Vec<&Operation> = self.gateway.operations().collect();
        let end = (start + PAGE_SIZE).min(ops.len());
        let page = ops[start.min(ops.len())..end]
            .iter()
            .copied()
            .map(operation_to_tool)
            .collect();
        let mut result = ListToolsResult::with_all_items(page);
        if end < ops.len() {
            result.next_cursor = Some(end.to_string());
        }
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let mapped = self.execute_named(&request.name, arguments).await;
        Ok(tool_result_to_mcp(&mapped).into())
    }

    async fn initialize(
        &self,
        _request: rmcp::model::InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::InitializeResult, McpError> {
        Ok(self.get_info())
    }

    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::DiscoverResult, McpError> {
        Ok(rmcp::model::DiscoverResult::from_server_info(
            self.supported_protocol_versions().into_owned(),
            self.get_info(),
        ))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, McpError> {
        Err(McpError::method_not_found::<
            rmcp::model::ListPromptsRequestMethod,
        >())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, McpError> {
        Err(McpError::method_not_found::<
            rmcp::model::ListResourcesRequestMethod,
        >())
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, McpError> {
        Err(McpError::method_not_found::<
            rmcp::model::ListResourceTemplatesRequestMethod,
        >())
    }

    async fn ping(&self, _context: RequestContext<RoleServer>) -> Result<(), McpError> {
        Ok(())
    }

    async fn complete(
        &self,
        _request: rmcp::model::CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CompleteResult, McpError> {
        Err(McpError::method_not_found::<
            rmcp::model::CompleteRequestMethod,
        >())
    }
}

fn operation_to_tool(op: &Operation) -> Tool {
    let mut t = Tool::new(
        op.tool.name.clone(),
        op.tool.description.clone(),
        Arc::new(
            op.tool
                .input_schema
                .as_object()
                .cloned()
                .unwrap_or_default(),
        ),
    );
    t.title = Some(op.tool.title.clone());
    if let Some(schema) = &op.tool.output_schema {
        t.output_schema = schema.as_object().cloned().map(Arc::new);
    }
    t
}

fn tool_result_to_mcp(result: &ToolResult) -> CallToolResult {
    let mut contents = vec![ContentBlock::text(result.text.clone())];
    if let Some(b64) = &result.image_b64 {
        contents.push(ContentBlock::image(b64.clone(), "image/png"));
    }
    let mut mapped = if result.is_error {
        CallToolResult::error(contents)
    } else {
        CallToolResult::success(contents)
    };
    mapped.structured_content = result.structured.clone();
    mapped
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_gateway_ir::IR_VERSION;

    #[test]
    fn unknown_tool_is_error() {
        let bundle = CompileBundle {
            api: mcp_gateway_ir::NormalizedApi {
                ir_version: IR_VERSION.to_owned(),
                gateway: mcp_gateway_ir::GatewayMeta {
                    title: "t".into(),
                    description: None,
                    spec_version: "3.0.0".into(),
                    source: mcp_gateway_ir::SourceMeta {
                        kind: mcp_gateway_ir::SourceKind::File,
                        locator: "spec.yaml".into(),
                        sha256: "a".repeat(64),
                    },
                },
                servers: vec![],
                security_schemes: Default::default(),
                operations: vec![],
            },
            report: mcp_gateway_ir::AnalysisReport {
                spec_title: "t".into(),
                spec_version: "3.0.0".into(),
                operations_total: 0,
                operations_compiled: 0,
                operations_skipped: 0,
                tools_enabled_by_default: 0,
                compile_ms: 0,
                warnings: vec![],
                skipped: vec![],
            },
        };
        let gateway = Arc::new(LocalGateway {
            bundle: Arc::new(bundle),
            base_url: "https://example.com".into(),
            credential: None,
            ssrf: SsrfPolicy::default(),
            enabled_tools: vec![],
            disabled_tools: vec![],
        });
        let handler = GatewayHandler::new(gateway, Arc::new(SequenceExecutor::from_responses([])));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(handler.execute_named("missing", serde_json::json!({})));
        assert!(result.is_error);
        assert_eq!(result.error_code.as_deref(), Some("unknown_tool"));
    }

    #[test]
    fn sequence_executor_is_fifo() {
        let exec = SequenceExecutor::from_responses([
            Ok(UpstreamResponse {
                status: 200,
                content_type: Some("application/json".into()),
                body: b"first".to_vec(),
            }),
            Ok(UpstreamResponse {
                status: 200,
                content_type: Some("application/json".into()),
                body: b"second".to_vec(),
            }),
        ]);
        let plan = mcp_gateway_ir::ExecutionPlan {
            method: "GET".into(),
            path_template: "/".into(),
            path_params: vec![],
            query_params: vec![],
            header_params: vec![],
            cookie_params: vec![],
            body: None,
            accept: "*/*".into(),
            timeout_ms: 5_000,
        };
        let req = render("https://example.com/", &plan, &serde_json::json!({})).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let a = rt
            .block_on(exec.execute(req.clone(), SsrfPolicy::default(), None))
            .unwrap();
        let b = rt
            .block_on(exec.execute(req, SsrfPolicy::default(), None))
            .unwrap();
        assert_eq!(a.body, b"first");
        assert_eq!(b.body, b"second");
    }
}
