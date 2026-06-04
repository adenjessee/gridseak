//! Global symbol table building from syntax results

use super::super::super::super::errors::ParsingError;
use super::super::super::super::ports::{GlobalSymbolTable, SyntaxResults};

/// Symbol table builder
pub struct SymbolTableBuilder;

impl SymbolTableBuilder {
    /// Build global symbol table from syntax results
    ///
    /// # Arguments
    /// * `syntax_results` - Syntax extraction results
    ///
    /// # Returns
    /// * `GlobalSymbolTable` - Built symbol table
    /// * `ParsingError` - If building fails
    pub fn build_from_syntax_results(
        syntax_results: &SyntaxResults,
    ) -> Result<GlobalSymbolTable, ParsingError> {
        let mut table = GlobalSymbolTable::new();

        // Aggregate symbols
        Self::aggregate_symbols(&mut table, syntax_results);

        // Aggregate call sites
        Self::aggregate_call_sites(&mut table, syntax_results);

        Ok(table)
    }

    /// Aggregate symbols into the symbol table
    fn aggregate_symbols(table: &mut GlobalSymbolTable, syntax_results: &SyntaxResults) {
        for symbol in &syntax_results.symbols {
            let file_path = symbol.location.file.clone();
            let symbol_info = crate::application::ports::SymbolInfo {
                id: symbol.id.clone(),
                name: symbol
                    .fqn
                    .split("::")
                    .last()
                    .unwrap_or(&symbol.fqn)
                    .to_string(),
                fqn: symbol.fqn.clone(),
                file: file_path,
                range: symbol.location.clone(),
                kind: symbol.kind,
                trait_metadata: symbol.trait_metadata.clone(),
            };
            table.add_symbol(symbol_info);
        }
    }

    /// Aggregate call sites into the symbol table. Aggregates every
    /// `UnresolvedReference` variant through its underlying
    /// [`CallSite`], so framework and declarative bindings show up in
    /// the symbol table alongside plain calls.
    fn aggregate_call_sites(table: &mut GlobalSymbolTable, syntax_results: &SyntaxResults) {
        for reference in &syntax_results.references {
            let call_site = reference.call_site();
            table.add_call_site(call_site.location.file.clone(), call_site.clone());
        }
    }
}
