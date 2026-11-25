//! The tool definitions (R8, KTD2): each schema is a JSON Schema object,
//! and a change to a definition must update its stored copy in
//! `crates/note-tools/schemas/`. To write the copies again, run this test
//! with `LONGHAND_UPDATE_SCHEMAS=1`.

use std::path::Path;

use note_tools::NoteTools;
use serde_json::Value;

const UPDATE: &str = "LONGHAND_UPDATE_SCHEMAS";

#[test]
fn each_definition_has_object_schemas_and_matches_its_stored_copy() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas");
    let definitions = NoteTools::new(20_000).definitions();
    assert_eq!(definitions.len(), 2);
    for definition in definitions {
        for schema in [&definition.input_schema, &definition.output_schema] {
            assert_eq!(schema["type"], "object", "{}", definition.name);
            assert!(
                schema["$schema"]
                    .as_str()
                    .is_some_and(|s| s.contains("json-schema.org")),
                "{}",
                definition.name
            );
        }
        let value = serde_json::to_value(&definition).unwrap();
        let file = folder.join(format!("{}.json", definition.name));
        let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
        if std::env::var_os(UPDATE).is_some() {
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(&file, &text).unwrap();
            continue;
        }
        let stored: Value = serde_json::from_str(
            &std::fs::read_to_string(&file)
                .unwrap_or_else(|_| panic!("{} is missing: run with {UPDATE}=1", file.display())),
        )
        .unwrap();
        assert_eq!(
            stored, value,
            "the definition of {} changed: check the change, then run with {UPDATE}=1",
            definition.name
        );
    }
}
