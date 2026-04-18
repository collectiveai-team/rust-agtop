//! Gemini CLI provider — stub.
use crate::error::Result;
use crate::pricing::Plan;
use crate::provider::Provider;
use crate::session::{ProviderKind, SessionAnalysis, SessionSummary};

#[derive(Debug, Default, Clone)]
pub struct GeminiCliProvider;

impl Provider for GeminiCliProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::GeminiCli
    }
    fn display_name(&self) -> &'static str {
        "Gemini CLI"
    }
    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        Ok(vec![])
    }
    fn analyze(&self, _summary: &SessionSummary, _plan: Plan) -> Result<SessionAnalysis> {
        Err(crate::error::Error::NoUsage("stub".to_string()))
    }
}
