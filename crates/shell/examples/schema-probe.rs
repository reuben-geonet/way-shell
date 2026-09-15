//! Installation-check artifact. Packaging copies this outside the application payload.
use gio::prelude::*;
use std::process::ExitCode;

#[derive(Debug, PartialEq, Eq)]
struct Report {
    schema: String,
    keys: usize,
}

fn probe(
    source: Option<&gio::SettingsSchemaSource>,
    backend: &gio::SettingsBackend,
    schemas: &[String],
) -> Result<Vec<Report>, String> {
    if schemas.is_empty() {
        return Err("At least one schema ID is required".into());
    }
    let source = source.ok_or("No default GSettings schema source is available")?;
    let backend_name = backend.type_().name();
    if backend_name != "GMemorySettingsBackend" {
        return Err(format!(
            "Expected GMemorySettingsBackend, found {backend_name}; set GSETTINGS_BACKEND=memory"
        ));
    }
    schemas
        .iter()
        .map(|id| {
            let schema = source
                .lookup(id, true)
                .ok_or_else(|| format!("Missing schema: {id}"))?;
            if schema.path().is_none() {
                return Err(format!("Schema requires a settings path: {id}"));
            }
            let settings = gio::Settings::new_full(&schema, Some(backend), None);
            let keys = schema.list_keys();
            for key in &keys {
                // GIO returns an owned, non-null Variant or reports an error.
                // Read every key in each requested schema.
                let _value = settings.value(key);
            }
            Ok(Report {
                schema: id.clone(),
                keys: keys.len(),
            })
        })
        .collect()
}

fn main() -> ExitCode {
    let schemas: Vec<String> = std::env::args().skip(1).collect();
    if schemas.is_empty() {
        eprintln!("Usage: schema-probe SCHEMA_ID [SCHEMA_ID ...]");
        return ExitCode::from(2);
    }
    let source = gio::SettingsSchemaSource::default();
    let backend = gio::SettingsBackend::default();
    match probe(source.as_ref(), &backend, &schemas) {
        Ok(reports) => {
            for report in reports {
                println!(
                    "Discovered and read {} ({} keys)",
                    report.schema, report.keys
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> gio::SettingsSchemaSource {
        gio::SettingsSchemaSource::from_directory(env!("WAY_SHELL_TEST_SCHEMAS"), None, false)
            .unwrap()
    }

    #[test]
    fn discovers_every_application_schema_and_reads_every_key() {
        let source = source();
        let (schemas, relocatable) = source.list_schemas(false);
        assert!(relocatable.is_empty());
        let schemas: Vec<_> = schemas.into_iter().map(String::from).collect();
        let reports = probe(Some(&source), &gio::memory_settings_backend_new(), &schemas).unwrap();
        assert_eq!(reports.len(), schemas.len());
        assert!(
            reports
                .iter()
                .any(|report| report.schema == "org.ldelossa.way-shell.system" && report.keys > 0)
        );
        for report in reports {
            assert_eq!(
                report.keys,
                source
                    .lookup(&report.schema, false)
                    .unwrap()
                    .list_keys()
                    .len()
            );
        }
    }

    #[test]
    fn missing_schema_and_missing_source_are_errors() {
        let backend = gio::memory_settings_backend_new();
        let missing = ["org.ldelossa.way-shell.nonexistent-probe-schema".into()];
        assert_eq!(
            probe(Some(&source()), &backend, &missing).unwrap_err(),
            format!("Missing schema: {}", missing[0])
        );
        assert!(
            probe(None, &backend, &missing)
                .unwrap_err()
                .contains("No default GSettings schema source")
        );
    }

    #[test]
    fn empty_requests_and_non_memory_backends_are_rejected() {
        let source = source();
        assert_eq!(
            probe(Some(&source), &gio::memory_settings_backend_new(), &[]).unwrap_err(),
            "At least one schema ID is required"
        );
        let error = probe(
            Some(&source),
            &gio::null_settings_backend_new(),
            &["org.ldelossa.way-shell.system".into()],
        )
        .unwrap_err();
        assert!(error.contains("Expected GMemorySettingsBackend"));
        assert!(error.contains("GNullSettingsBackend"));
    }
}
