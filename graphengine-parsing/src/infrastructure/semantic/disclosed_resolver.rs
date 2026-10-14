//! Thin wrapper that attaches a fixed [`ResolutionDisclosure`] to any resolver.

use crate::application::ports::{
    ResolvedEdges, SemanticResolver, SessionMetricsSnapshot, SyntaxResults,
};
use crate::application::resolution_disclosure::ResolutionDisclosure;
use async_trait::async_trait;

pub struct DisclosedSemanticResolver {
    inner: Box<dyn SemanticResolver>,
    disclosure: ResolutionDisclosure,
    /// Report the inner resolver's emitted-edge count (when it still emits).
    keep_inner_edges: bool,
}

impl DisclosedSemanticResolver {
    pub fn new(inner: Box<dyn SemanticResolver>, disclosure: ResolutionDisclosure) -> Self {
        Self {
            inner,
            disclosure,
            keep_inner_edges: false,
        }
    }

    /// Like [`Self::new`], but `emitted_edges` comes from the inner
    /// resolver's own disclosure (a fallback tier that still emits edges).
    pub fn keeping_inner_edges(
        inner: Box<dyn SemanticResolver>,
        disclosure: ResolutionDisclosure,
    ) -> Self {
        Self {
            inner,
            disclosure,
            keep_inner_edges: true,
        }
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
        let mut d = self.disclosure.clone();
        if self.keep_inner_edges {
            d.emitted_edges = self
                .inner
                .resolution_disclosure()
                .await
                .and_then(|inner| inner.emitted_edges);
        }
        Some(d)
    }

    async fn layer2_telemetry_json(&self) -> Option<String> {
        self.inner.layer2_telemetry_json().await
    }
}
