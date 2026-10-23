//! Embedded, versioned test catalog shared by the API and the TypeScript worker.
//!
//! The JSON file is server-owned and contains no executable fields. The API
//! embeds it at compile time; a unit test fails if it drifts from the catalog
//! schema or contains invalid bounds.

use std::sync::OnceLock;

use probeforge_contracts::Risk;
use serde::{Deserialize, Serialize};

const RAW_CATALOG: &str = include_str!("../../../packages/test-catalog/catalog.json");
const SCHEMA_VERSION: &str = "1";
const MAX_ENTRIES: usize = 500;
const MAX_ID_LENGTH: usize = 120;
const MAX_NAME_LENGTH: usize = 160;
const MAX_DESCRIPTION_LENGTH: usize = 2_000;
const MAX_CATALOG_VERSION_LENGTH: usize = 64;
const MAX_ARRAY_ITEMS: usize = 20;
const MAX_PREREQUISITE_LENGTH: usize = 300;
const MAX_TOOL_REQUIREMENT_LENGTH: usize = 200;
const MAX_TARGET_REQUIREMENT_LENGTH: usize = 300;
const MAX_EVIDENCE_EXPECTATION_LENGTH: usize = 500;
const MAX_TIMEOUT_SECONDS: i32 = 3_600;

/// Execution adapters are not implemented until Phase 10. Keeping this list
/// empty makes the API report every catalog entry as unavailable instead of
/// pretending that approval can run it.
const AVAILABLE_TOOL_REQUIREMENTS: &[&str] = &[];
const UNAVAILABLE_NOTE: &str = "Execution adapter is not implemented yet. Approval records the selected test; it does not start a run.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogCategory {
    Static,
    Http,
    Network,
    Load,
}

impl CatalogCategory {
    fn is_valid(self) -> bool {
        matches!(self, Self::Static | Self::Http | Self::Network | Self::Load)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogEntry {
    id: String,
    name: String,
    category: CatalogCategory,
    risk: Risk,
    description: String,
    prerequisites: Vec<String>,
    tool_requirements: Vec<String>,
    target_requirements: Vec<String>,
    timeout_seconds: i32,
    evidence_expectations: Vec<String>,
    requires_explicit_confirmation: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestCatalog {
    schema_version: String,
    catalog_version: String,
    entries: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntryDto {
    pub id: String,
    pub name: String,
    pub category: CatalogCategory,
    pub risk: Risk,
    pub description: String,
    pub prerequisites: Vec<String>,
    pub tool_requirements: Vec<String>,
    pub target_requirements: Vec<String>,
    pub timeout_seconds: i32,
    pub evidence_expectations: Vec<String>,
    pub requires_explicit_confirmation: bool,
    pub available: bool,
    pub availability_note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCatalogDto {
    pub schema_version: String,
    pub catalog_version: String,
    pub entries: Vec<CatalogEntryDto>,
}

fn has_control_characters(value: &str) -> bool {
    value.chars().any(|character| character.is_control())
}

fn is_bounded_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !has_control_characters(value)
}

fn validate_list(
    values: &[String],
    maximum_items: usize,
    maximum_length: usize,
    allow_empty: bool,
    field: &str,
) -> Result<(), String> {
    if values.len() > maximum_items || (!allow_empty && values.is_empty()) {
        return Err(format!("{field} has an invalid item count"));
    }
    for value in values {
        if !is_bounded_text(value, maximum_length) {
            return Err(format!("{field} contains an invalid string"));
        }
    }
    Ok(())
}

fn validate_catalog(catalog: &TestCatalog) -> Result<(), String> {
    if catalog.schema_version != SCHEMA_VERSION {
        return Err("catalog schemaVersion must be 1".to_owned());
    }
    if !is_bounded_text(&catalog.catalog_version, MAX_CATALOG_VERSION_LENGTH) {
        return Err("catalog catalogVersion is invalid".to_owned());
    }
    if catalog.entries.is_empty() || catalog.entries.len() > MAX_ENTRIES {
        return Err("catalog must contain a bounded non-empty entry list".to_owned());
    }

    let mut ids = std::collections::HashSet::new();
    for entry in &catalog.entries {
        if !entry.id.starts_with(|character: char| {
            character.is_ascii_lowercase() || character.is_ascii_digit()
        }) || entry.id.len() < 2
            || entry.id.len() > MAX_ID_LENGTH
            || !entry.id.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || character == '.'
                    || character == '-'
            })
        {
            return Err(format!("catalog entry id {:?} is invalid", entry.id));
        }
        if !ids.insert(entry.id.clone()) {
            return Err(format!("catalog entry id {:?} is duplicated", entry.id));
        }
        if !is_bounded_text(&entry.name, MAX_NAME_LENGTH) {
            return Err(format!("catalog entry {:?} has an invalid name", entry.id));
        }
        if !is_bounded_text(&entry.description, MAX_DESCRIPTION_LENGTH) {
            return Err(format!(
                "catalog entry {:?} has an invalid description",
                entry.id
            ));
        }
        if !entry.category.is_valid() {
            return Err(format!(
                "catalog entry {:?} has an invalid category",
                entry.id
            ));
        }
        if !matches!(entry.risk, Risk::Low | Risk::Medium | Risk::High) {
            return Err(format!("catalog entry {:?} has an invalid risk", entry.id));
        }
        validate_list(
            &entry.prerequisites,
            MAX_ARRAY_ITEMS,
            MAX_PREREQUISITE_LENGTH,
            true,
            "prerequisites",
        )?;
        validate_list(
            &entry.tool_requirements,
            10,
            MAX_TOOL_REQUIREMENT_LENGTH,
            false,
            "toolRequirements",
        )?;
        validate_list(
            &entry.target_requirements,
            10,
            MAX_TARGET_REQUIREMENT_LENGTH,
            false,
            "targetRequirements",
        )?;
        validate_list(
            &entry.evidence_expectations,
            MAX_ARRAY_ITEMS,
            MAX_EVIDENCE_EXPECTATION_LENGTH,
            false,
            "evidenceExpectations",
        )?;
        if !(1..=MAX_TIMEOUT_SECONDS).contains(&entry.timeout_seconds) {
            return Err(format!(
                "catalog entry {:?} has an invalid timeout",
                entry.id
            ));
        }
    }
    Ok(())
}

fn is_entry_available(entry: &CatalogEntry) -> bool {
    entry
        .tool_requirements
        .iter()
        .all(|requirement| AVAILABLE_TOOL_REQUIREMENTS.contains(&requirement.as_str()))
}

fn to_dto(entry: &CatalogEntry) -> CatalogEntryDto {
    let available = is_entry_available(entry);
    CatalogEntryDto {
        id: entry.id.clone(),
        name: entry.name.clone(),
        category: entry.category,
        risk: entry.risk,
        description: entry.description.clone(),
        prerequisites: entry.prerequisites.clone(),
        tool_requirements: entry.tool_requirements.clone(),
        target_requirements: entry.target_requirements.clone(),
        timeout_seconds: entry.timeout_seconds,
        evidence_expectations: entry.evidence_expectations.clone(),
        requires_explicit_confirmation: entry.requires_explicit_confirmation,
        available,
        availability_note: (!available).then(|| UNAVAILABLE_NOTE.to_owned()),
    }
}

fn catalog_result() -> &'static Result<TestCatalog, String> {
    static CATALOG: OnceLock<Result<TestCatalog, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let parsed: TestCatalog =
            serde_json::from_str(RAW_CATALOG).map_err(|error| error.to_string())?;
        validate_catalog(&parsed)?;
        Ok(parsed)
    })
}

fn catalog() -> Result<&'static TestCatalog, &'static str> {
    match catalog_result() {
        Ok(catalog) => Ok(catalog),
        Err(error) => Err(error.as_str()),
    }
}

pub fn catalog_dto() -> Result<TestCatalogDto, &'static str> {
    let catalog = catalog()?;
    Ok(TestCatalogDto {
        schema_version: catalog.schema_version.clone(),
        catalog_version: catalog.catalog_version.clone(),
        entries: catalog.entries.iter().map(to_dto).collect(),
    })
}

pub fn catalog_version() -> Result<&'static str, &'static str> {
    catalog().map(|catalog| catalog.catalog_version.as_str())
}

pub fn find_entry(catalog_test_id: &str) -> Result<Option<CatalogEntryDto>, &'static str> {
    let catalog = catalog()?;
    Ok(catalog
        .entries
        .iter()
        .find(|entry| entry.id == catalog_test_id)
        .map(to_dto))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_is_valid_and_versioned() {
        let catalog = catalog().expect("embedded catalog must be valid");
        assert_eq!(catalog.schema_version, "1");
        assert_eq!(catalog.catalog_version, "0.2.0");
        assert!(catalog.entries.len() >= 6);
        assert!(
            catalog
                .entries
                .iter()
                .any(|entry| entry.category == CatalogCategory::Load)
        );
    }

    #[test]
    fn embedded_catalog_exposes_unavailable_tools_honestly() {
        let dto = catalog_dto().expect("catalog dto");
        assert!(dto.entries.iter().all(|entry| !entry.available));
        assert!(
            dto.entries
                .iter()
                .all(|entry| entry.availability_note.as_deref() == Some(UNAVAILABLE_NOTE))
        );
        assert_eq!(
            find_entry("http.headers.missing")
                .expect("catalog lookup")
                .expect("known entry")
                .risk,
            Risk::Low
        );
        assert!(
            find_entry("http.invented")
                .expect("catalog lookup")
                .is_none()
        );
    }

    #[test]
    fn catalog_rejects_duplicate_or_unbounded_entries() {
        let duplicate = TestCatalog {
            schema_version: "1".to_owned(),
            catalog_version: "test".to_owned(),
            entries: vec![
                CatalogEntry {
                    id: "http.test".to_owned(),
                    name: "Test".to_owned(),
                    category: CatalogCategory::Http,
                    risk: Risk::Low,
                    description: "A test fixture.".to_owned(),
                    prerequisites: Vec::new(),
                    tool_requirements: vec!["tool>=1".to_owned()],
                    target_requirements: vec!["target".to_owned()],
                    timeout_seconds: 1,
                    evidence_expectations: vec!["evidence".to_owned()],
                    requires_explicit_confirmation: false,
                },
                CatalogEntry {
                    id: "http.test".to_owned(),
                    name: "Test".to_owned(),
                    category: CatalogCategory::Http,
                    risk: Risk::Low,
                    description: "A test fixture.".to_owned(),
                    prerequisites: Vec::new(),
                    tool_requirements: vec!["tool>=1".to_owned()],
                    target_requirements: vec!["target".to_owned()],
                    timeout_seconds: 1,
                    evidence_expectations: vec!["evidence".to_owned()],
                    requires_explicit_confirmation: false,
                },
            ],
        };
        assert!(validate_catalog(&duplicate).is_err());
    }
}
