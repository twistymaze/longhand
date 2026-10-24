//! One entry of the import record: what one computer did with one file.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::store::ComputerId;

/// The schema version that this build writes. A write keeps a higher
/// version that is in the file.
pub const SCHEMA_VERSION: u64 = 1;

/// What a computer did with a file. A lookup resolves `Imported` over
/// `Skipped` over `Failed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// The decoder refused the complete file (KTD11). The lowest rank.
    Failed,
    /// The user or the app chose not to import the file.
    Skipped,
    /// The file is a note. The highest rank.
    Imported,
}

/// The contents of one file `imports/<sha256>/<computer_id>.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub schema_version: u64,
    pub state: State,
    /// The name of the imported file, with no folder.
    pub source_name: String,
    /// The size of the imported file in bytes.
    pub size: u64,
    /// The id of the computer that wrote the entry.
    pub computer_id: String,
    /// The time of the write.
    pub time: DateTime<FixedOffset>,
    /// For `Failed`: the Longhand version that tried the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longhand_version: Option<String>,
    /// For `Failed`: the name of the decoder that refused the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder_name: Option<String>,
    /// For `Failed`: the version of the decoder that refused the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder_version: Option<String>,
    /// The fields that this build does not know. A write keeps them.
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Entry {
    /// An entry of this build with the state `state` and no failure fields.
    pub fn new(
        state: State,
        source_name: &str,
        size: u64,
        computer_id: ComputerId,
        time: DateTime<FixedOffset>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            state,
            source_name: source_name.to_string(),
            size,
            computer_id: computer_id.as_str().to_string(),
            time,
            longhand_version: None,
            decoder_name: None,
            decoder_version: None,
            other: Map::new(),
        }
    }

    /// A `Failed` entry of this build (KTD11).
    pub fn failed(
        source_name: &str,
        size: u64,
        computer_id: ComputerId,
        time: DateTime<FixedOffset>,
        longhand_version: &str,
        decoder_name: &str,
        decoder_version: &str,
    ) -> Self {
        Self {
            longhand_version: Some(longhand_version.to_string()),
            decoder_name: Some(decoder_name.to_string()),
            decoder_version: Some(decoder_version.to_string()),
            ..Self::new(State::Failed, source_name, size, computer_id, time)
        }
    }

    /// Whether the Longhand version `running` tries the file of this
    /// `Failed` entry again (R12a): only a version newer than the version
    /// in the entry does. An entry with no version, or with a version that
    /// does not parse, gives no retry.
    pub fn allows_retry(&self, running: &str) -> bool {
        let Some(recorded) = self.longhand_version.as_deref().and_then(version_parts) else {
            return false;
        };
        version_parts(running).is_some_and(|running| running > recorded)
    }

    /// The Longhand version of the entry as numbers, for the comparison of
    /// two `Failed` entries.
    pub(crate) fn version(&self) -> Option<Vec<u64>> {
        self.longhand_version.as_deref().and_then(version_parts)
    }
}

/// The numbers of the version `text`, such as `[1, 5, 0]` for `1.5.0`.
/// A pre-release or build suffix after `-` or `+` is not used. Trailing
/// zeros are removed, so `1.5` and `1.5.0` are equal.
fn version_parts(text: &str) -> Option<Vec<u64>> {
    let core = text.split(['-', '+']).next()?;
    let mut parts = core
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    while parts.last() == Some(&0) {
        parts.pop();
    }
    Some(parts)
}
