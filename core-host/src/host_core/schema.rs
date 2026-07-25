use super::*;

pub(crate) const INTEGRITY_CONFIG_SCHEMA_FILE: &str = "integrity-config.schema.json";
pub(crate) const INTEGRITY_LOCK_SCHEMA_FILE: &str = "integrity-lock.schema.json";

pub(crate) fn integrity_config_schema(schema_id: Option<&str>) -> serde_json::Value {
    schema_with_id(
        serde_json::to_value(schemars::schema_for!(IntegrityConfig))
            .expect("IntegrityConfig JSON Schema should serialize"),
        schema_id,
    )
}

pub(crate) fn integrity_manifest_schema(schema_id: Option<&str>) -> serde_json::Value {
    schema_with_id(
        serde_json::to_value(schemars::schema_for!(IntegrityManifest))
            .expect("IntegrityManifest JSON Schema should serialize"),
        schema_id,
    )
}

pub(crate) fn release_schema_id(release_tag: &str, file_name: &str) -> String {
    format!("https://github.com/astorise/tachyon-mesh/releases/download/{release_tag}/{file_name}")
}

pub(crate) fn write_integrity_schema_files(command: &SchemaCommand) -> Result<()> {
    fs::create_dir_all(&command.output_dir).with_context(|| {
        format!(
            "failed to create schema output directory {}",
            command.output_dir.display()
        )
    })?;

    let config_id = command
        .release_tag
        .as_deref()
        .map(|tag| release_schema_id(tag, INTEGRITY_CONFIG_SCHEMA_FILE));
    let lock_id = command
        .release_tag
        .as_deref()
        .map(|tag| release_schema_id(tag, INTEGRITY_LOCK_SCHEMA_FILE));

    write_schema_file(
        &command.output_dir.join(INTEGRITY_CONFIG_SCHEMA_FILE),
        integrity_config_schema(config_id.as_deref()),
    )?;
    write_schema_file(
        &command.output_dir.join(INTEGRITY_LOCK_SCHEMA_FILE),
        integrity_manifest_schema(lock_id.as_deref()),
    )?;
    Ok(())
}

fn write_schema_file(path: &Path, schema: serde_json::Value) -> Result<()> {
    let data =
        serde_json::to_string_pretty(&schema).expect("JSON Schema document should serialize");
    fs::write(path, format!("{data}\n"))
        .with_context(|| format!("failed to write schema file {}", path.display()))
}

fn schema_with_id(mut schema: serde_json::Value, schema_id: Option<&str>) -> serde_json::Value {
    if let Some(schema_id) = schema_id {
        if let Some(object) = schema.as_object_mut() {
            object.insert(
                "$id".to_owned(),
                serde_json::Value::String(schema_id.to_owned()),
            );
        }
    }
    schema
}
