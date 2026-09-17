//! Thin wrapper that attaches a fixed [`ResolutionDisclosure`] to any resolver.

use crate::application::ports::{
    ResolvedEdges, SemanticResolver, SessionMetricsSnapshot, SyntaxResults,
};
use crate::application::resolution_disclosure::ResolutionDisclosure;
use async_trait::async_trait;

pub struct DisclosedSemanticResolver {
    inner: Box<dyn SemanticResolver>,
    disclosure: ResolutionDisclosure,
}

impl DisclosedSemanticResolver {
    pub fn new(inner: Box<dyn SemanticResolver>, disclosure: ResolutionDisclosure) -> Self {
        Self { inner, disclosure }
    }

    pub fn disclosure(&self) -> &ResolutionDisclosure {
        &self.disclosure
    }
}

#[async_trait]
impl SemanticResolver for DisclosedSemanticResolver {
    async fn resolve(&self, hints: &SyntaxResults) -> anyhow::Result<ResolvedEdges> {
        self.inner.resolve(hints).await
    }

    fn supported_language(&self) -> &str {
        self.inner.supported_language()
    }

    async fn is_available(&self) -> bool {
        self.inner.is_available().await
    }

    async fn session_metrics(&self) -> Option<SessionMetricsSnapshot> {
        self.inner.session_metrics().await
    }

    async fn resolution_disclosure(&self) -> Option<ResolutionDisclosure> {
        Some(self.disclosure.clone())
    }
}
