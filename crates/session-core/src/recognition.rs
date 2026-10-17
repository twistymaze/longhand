//! Portable recognition output. This module has no native runtime dependency.
use serde::{Deserialize, Serialize};
/// A half-open interval on the canonical recording timeline, in samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerScope {
    pub generation: String,
    pub epoch: u64,
}

/// Batch calls do not establish speaker continuity between input windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerKey {
    pub scope: SpeakerScope,
    pub input_start_sample: u64,
    pub local_id: u32,
}

/// A replay of one input with the same scope keeps each ordered row's identity.
/// A revision replaces that input's complete row list, including removed rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognitionId {
    pub scope: SpeakerScope,
    pub input_start_sample: u64,
    pub ordinal: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Finality {
    Provisional,
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimingPrecision {
    /// Model-provided segment bounds. These are not word alignments.
    NativeSegment,
    /// A native segment whose end was clipped by an explicit adapter tolerance.
    /// This does not improve or guarantee the model's timestamp accuracy.
    NativeSegmentClipped,
    /// A model timestamp with no known duration, including a clipped tail word.
    /// Its range is a single point and contributes no timed speech coverage.
    NativeTimePoint,
    /// Only the complete source window is known. Row order remains meaningful.
    SourceWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognitionSegment {
    pub id: RecognitionId,
    pub range: SampleRange,
    pub timing: TimingPrecision,
    pub text: String,
    pub speaker: Option<SpeakerKey>,
    pub revision: u64,
    pub text_finality: Finality,
    pub speaker_finality: Finality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerTurn {
    pub range: SampleRange,
    pub timing: TimingPrecision,
    pub speaker: SpeakerKey,
    pub finality: Finality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognitionOutput {
    /// Identifies the complete snapshot, including an empty replacement.
    pub scope: SpeakerScope,
    pub revision: u64,
    /// Input accepted by the completed call, including silence. This does not
    /// prove that the model decoded every spoken word or rule out truncation.
    pub coverage: SampleRange,
    pub text: String,
    pub segments: Vec<RecognitionSegment>,
    pub speaker_turns: Vec<SpeakerTurn>,
}

use crate::{Store, StoreError};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
};

pub const ARTIFACT_VERSION: u32 = 1;
pub const ARTIFACT_FILE: &str = "recognition.jsonl";
/// Maximum encoded size of one output block.
pub const MAX_BLOCK_BYTES: usize = 1024 * 1024;
/// Bound each transaction by record count and encoded payload size.
pub const MAX_COMMIT_BLOCKS: usize = 128;
pub const MAX_COMMIT_BYTES: usize = MAX_BLOCK_BYTES;
pub type ArtifactResult<T> = Result<T, ArtifactError>;
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid recognition artifact: {0}")]
    Invalid(String),
}
fn invalid(message: &str) -> ArtifactError {
    ArtifactError::Invalid(message.into())
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn file_digest(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprints {
    pub execution: String,
    pub asr: String,
    pub speakers: String,
    pub audio: String,
    pub preprocessing: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reuse {
    Complete,
    TimedAsr,
    TranscribeAgain,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactHeader {
    pub version: u32,
    pub generation: String,
    pub snapshot: serde_json::Value,
    pub fingerprints: Fingerprints,
    pub transcript: String,
    pub limitations: Vec<String>,
    pub coverage: Vec<SampleRange>,
}
impl ArtifactHeader {
    pub fn reuse(&self, next: &Fingerprints, transcript: &[u8]) -> Reuse {
        if self.version != ARTIFACT_VERSION
            || digest(transcript) != self.transcript
            || self.fingerprints.asr != next.asr
            || self.fingerprints.audio != next.audio
            || self.fingerprints.preprocessing != next.preprocessing
        {
            return Reuse::TranscribeAgain;
        }
        if &self.fingerprints == next {
            Reuse::Complete
        } else {
            Reuse::TimedAsr
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactReference {
    pub version: u32,
    pub filename: String,
    pub sha256: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub received: Option<SampleRange>,
    pub computed: Option<SampleRange>,
    pub provisional: Option<SampleRange>,
    pub committed: Option<SampleRange>,
}
fn contains(outer: Option<SampleRange>, inner: Option<SampleRange>) -> bool {
    inner.is_none_or(|i| outer.is_some_and(|o| o.start <= i.start && o.end >= i.end))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputBlock {
    pub key: String,
    pub scope: SpeakerScope,
    pub revision: u64,
    pub coverage: Coverage,
    pub rows: Vec<RecognitionSegment>,
    pub speakers: Vec<SpeakerTurn>,
}
impl OutputBlock {
    pub fn validate(&self) -> ArtifactResult<()> {
        if self.key.is_empty()
            || self.scope.generation.is_empty()
            || self.revision > i64::MAX as u64
        {
            return Err(invalid("missing output identity"));
        }
        let c = &self.coverage;
        if [c.received, c.computed, c.provisional, c.committed]
            .into_iter()
            .flatten()
            .any(|r| r.end < r.start)
            || !contains(c.received, c.computed)
            || !contains(c.computed, c.provisional)
            || !contains(c.computed, c.committed)
        {
            return Err(invalid("coverage order is invalid"));
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.id.scope != self.scope
                || row.revision > self.revision
                || row.range.end < row.range.start
                || (row.range.end == row.range.start
                    && row.timing != TimingPrecision::NativeTimePoint)
                || self.rows[..index].iter().any(|old| old.id == row.id)
                || !contains(c.computed, Some(row.range))
                || row
                    .speaker
                    .as_ref()
                    .is_some_and(|s| s.scope.generation != self.scope.generation)
                || (c.committed.is_some()
                    && (row.text_finality != Finality::Final
                        || row.speaker_finality != Finality::Final))
            {
                return Err(invalid("row identity, coverage or finality is invalid"));
            }
        }
        if self.speakers.iter().any(|s| {
            s.speaker.scope.generation != self.scope.generation
                || s.range.end < s.range.start
                || !contains(c.computed, Some(s.range))
                || (c.committed.is_some() && s.finality != Finality::Final)
        }) {
            return Err(invalid("speaker coverage or finality is invalid"));
        }
        if serde_json::to_vec(self)?.len() > MAX_BLOCK_BYTES {
            return Err(invalid("output block exceeds size limit"));
        }
        Ok(())
    }
    pub fn timed_asr(&self) -> bool {
        self.coverage.committed.is_some()
            && self.rows.iter().all(|r| {
                r.timing != TimingPrecision::SourceWindow && r.text_finality == Finality::Final
            })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    pub replay_from: u64,
    pub scope: SpeakerScope,
    pub native_state_restored: bool,
}
impl Store {
    /// Output, revision, namespace and coverage commit in one SQLite transaction.
    pub fn commit_recognition(
        &mut self,
        id: &str,
        block: &OutputBlock,
    ) -> crate::jobs::Result<bool> {
        Ok(self.commit_recognition_batch(id, std::slice::from_ref(block))? != 0)
    }
    /// Commit a bounded batch atomically. Conflicts roll back the whole batch.
    pub fn commit_recognition_batch(
        &mut self,
        id: &str,
        blocks: &[OutputBlock],
    ) -> crate::jobs::Result<usize> {
        if blocks.len() > MAX_COMMIT_BLOCKS {
            return Err(StoreError::Recognition(
                "output batch exceeds record limit".into(),
            ));
        }
        let mut payloads = Vec::with_capacity(blocks.len());
        let mut bytes = 0;
        for block in blocks {
            block
                .validate()
                .map_err(|e| StoreError::Recognition(e.to_string()))?;
            let payload = serde_json::to_string(block)?;
            bytes += payload.len();
            if bytes > MAX_COMMIT_BYTES {
                return Err(StoreError::Recognition(
                    "output batch exceeds size limit".into(),
                ));
            }
            payloads.push(payload);
        }
        if blocks.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        for (block, payload) in blocks.iter().zip(payloads) {
            let old: Option<String> = tx.query_row("SELECT payload FROM recognition_output WHERE session_id=?1 AND generation=?2 AND key=?3", params![id,block.scope.generation,block.key], |r| r.get(0)).optional()?;
            if let Some(old) = old {
                let old: OutputBlock = serde_json::from_str(&old)?;
                if block.revision < old.revision || block == &old {
                    continue;
                }
                if block.revision == old.revision
                    || block.scope != old.scope
                    || !contains(block.coverage.committed, old.coverage.committed)
                    || (old.coverage.committed.is_some()
                        && (block.rows != old.rows || block.speakers != old.speakers))
                {
                    return Err(StoreError::Recognition(
                        "revision conflicts with durable output".into(),
                    ));
                }
            }
            tx.execute("INSERT INTO recognition_output(session_id,generation,key,revision,payload) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(session_id,generation,key) DO UPDATE SET revision=excluded.revision,payload=excluded.payload",params![id,block.scope.generation,block.key,block.revision as i64,payload])?;
            changed += 1;
        }
        tx.commit()?;
        Ok(changed)
    }
    pub fn recognition_block(
        &self,
        id: &str,
        generation: &str,
        key: &str,
    ) -> crate::jobs::Result<Option<OutputBlock>> {
        let value: Option<String> = self.conn.query_row("SELECT payload FROM recognition_output WHERE session_id=?1 AND generation=?2 AND key=?3",params![id,generation,key],|r|r.get(0)).optional()?;
        value
            .map(|s| {
                let block: OutputBlock = serde_json::from_str(&s)?;
                block
                    .validate()
                    .map_err(|e| StoreError::Recognition(e.to_string()))?;
                Ok(block)
            })
            .transpose()
    }
    /// Iterate one bounded block at a time. The consumer owns no session-wide cache.
    pub fn visit_recognition(
        &self,
        id: &str,
        generation: &str,
        mut visitor: impl FnMut(OutputBlock) -> crate::jobs::Result<()>,
    ) -> crate::jobs::Result<()> {
        let mut stmt = self.conn.prepare("SELECT payload FROM recognition_output WHERE session_id=?1 AND generation=?2 ORDER BY key")?;
        let mut rows = stmt.query(params![id, generation])?;
        while let Some(row) = rows.next()? {
            let payload: String = row.get(0)?;
            let block: OutputBlock = serde_json::from_str(&payload)?;
            block
                .validate()
                .map_err(|e| StoreError::Recognition(e.to_string()))?;
            visitor(block)?;
        }
        Ok(())
    }
    /// A saved cursor is progress evidence only. Every restarted native stream
    /// replays from its epoch start and obtains a new speaker namespace.
    pub fn restart_recognition_epoch(
        &mut self,
        id: &str,
        generation: &str,
        epoch_start: u64,
        received_end: u64,
    ) -> crate::jobs::Result<Replay> {
        if received_end < epoch_start {
            return Err(StoreError::Recognition("invalid replay interval".into()));
        }
        let tx = self.conn.transaction()?;
        let epoch: i64 = tx.query_row("INSERT INTO recognition_epochs(session_id,generation,next_epoch) VALUES (?1,?2,1) ON CONFLICT(session_id,generation) DO UPDATE SET next_epoch=next_epoch+1 RETURNING next_epoch",params![id,generation],|r|r.get(0))?;
        tx.commit()?;
        Ok(Replay {
            replay_from: epoch_start,
            scope: SpeakerScope {
                generation: generation.into(),
                epoch: u64::try_from(epoch)
                    .map_err(|_| StoreError::Recognition("invalid epoch".into()))?,
            },
            native_state_restored: false,
        })
    }
}
fn write_line(writer: &mut impl Write, value: &impl Serialize) -> ArtifactResult<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_BLOCK_BYTES {
        return Err(invalid("artifact record exceeds size limit"));
    }
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    Ok(())
}
pub fn write_artifact(
    path: &Path,
    header: &ArtifactHeader,
    blocks: impl IntoIterator<Item = OutputBlock>,
) -> ArtifactResult<ArtifactReference> {
    if header.version != ARTIFACT_VERSION {
        return Err(invalid("unsupported version"));
    }
    let temporary = path.with_extension("jsonl.tmp");
    let mut file = File::create(&temporary)?;
    write_line(&mut file, header)?;
    for block in blocks {
        block.validate()?;
        if block.scope.generation != header.generation || block.coverage.committed.is_none() {
            return Err(invalid("artifact contains uncommitted output"));
        }
        write_line(&mut file, &block)?;
    }
    file.sync_all()?;
    let sha256 = file_digest(&temporary)?;
    std::fs::rename(&temporary, path)?;
    #[cfg(unix)]
    File::open(
        path.parent()
            .ok_or_else(|| invalid("artifact has no directory"))?,
    )?
    .sync_all()?;
    Ok(ArtifactReference {
        version: ARTIFACT_VERSION,
        filename: path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| invalid("invalid filename"))?
            .into(),
        sha256,
    })
}
fn line(reader: &mut impl BufRead) -> ArtifactResult<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_BLOCK_BYTES as u64 + 2)
        .read_until(b'\n', &mut bytes)?;
    if bytes.len() > MAX_BLOCK_BYTES + 1 || (!bytes.is_empty() && bytes.last() != Some(&b'\n')) {
        return Err(invalid("artifact record is oversized or incomplete"));
    }
    Ok(bytes)
}
pub fn read_artifact(
    path: &Path,
    reference: &ArtifactReference,
    mut visitor: impl FnMut(OutputBlock) -> ArtifactResult<()>,
) -> ArtifactResult<ArtifactHeader> {
    use std::io::{Seek, SeekFrom};
    if reference.version != ARTIFACT_VERSION
        || reference.filename != ARTIFACT_FILE
        || path.file_name().and_then(|n| n.to_str()) != Some(reference.filename.as_str())
    {
        return Err(invalid("unsupported version or artifact filename"));
    }
    // Copy to an unnamed file before verification. The reader then consumes the
    // exact verified bytes even if another process replaces or edits the source.
    let mut source = File::open(path)?;
    let mut stable = tempfile::tempfile()?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        stable.write_all(&buffer[..count])?;
    }
    if format!("{:x}", hash.finalize()) != reference.sha256 {
        return Err(invalid("checksum mismatch"));
    }
    stable.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::new(stable);
    let header: ArtifactHeader = serde_json::from_slice(&line(&mut reader)?)?;
    if header.version != ARTIFACT_VERSION || header.generation.is_empty() {
        return Err(invalid("unsupported version or generation"));
    }
    // SQLite keeps validation metadata on disk. This detects duplicate output
    // identities and coverage holes without retaining all records in memory.
    let mut validation = rusqlite::Connection::open("")?;
    validation.execute_batch("CREATE TABLE ranges(key TEXT PRIMARY KEY,start INTEGER NOT NULL,end INTEGER NOT NULL); CREATE INDEX range_start ON ranges(start,end);")?;
    let transaction = validation.transaction()?;
    loop {
        let bytes = line(&mut reader)?;
        if bytes.is_empty() {
            break;
        }
        let block: OutputBlock = serde_json::from_slice(&bytes)?;
        block.validate()?;
        if block.scope.generation != header.generation || block.coverage.committed.is_none() {
            return Err(invalid("artifact contains uncommitted output"));
        }
        let range = block.coverage.committed.expect("checked committed range");
        if !header
            .coverage
            .iter()
            .any(|outer| outer.start <= range.start && outer.end >= range.end)
        {
            return Err(invalid("output is outside artifact coverage"));
        }
        let start = i64::try_from(range.start)
            .map_err(|_| invalid("sample offset exceeds storage capacity"))?;
        let end = i64::try_from(range.end)
            .map_err(|_| invalid("sample offset exceeds storage capacity"))?;
        transaction.execute(
            "INSERT INTO ranges(key,start,end) VALUES (?1,?2,?3)",
            params![block.key, start, end],
        )?;
    }
    transaction.commit()?;
    for required in &header.coverage {
        if required.end < required.start {
            return Err(invalid("invalid artifact coverage"));
        }
        let mut at = required.start;
        let mut query = validation.prepare("SELECT start,end FROM ranges ORDER BY start,end")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let start: i64 = row.get(0)?;
            let end: i64 = row.get(1)?;
            let start = start as u64;
            let end = end as u64;
            if start > at {
                break;
            }
            at = at.max(end);
            if at >= required.end {
                break;
            }
        }
        if at < required.end {
            return Err(invalid("artifact has a committed coverage hole"));
        }
    }
    reader.seek(SeekFrom::Start(0))?;
    line(&mut reader)?;
    loop {
        let bytes = line(&mut reader)?;
        if bytes.is_empty() {
            break;
        }
        visitor(serde_json::from_slice(&bytes)?)?;
    }
    Ok(header)
}

impl Store {
    /// Bounded keyset pages for native correction state. Prefixes are internal
    /// generated keys. Escape SQL pattern characters before matching them.
    pub fn recognition_page(
        &self,
        id: &str,
        generation: &str,
        prefix: &str,
        after: &str,
    ) -> crate::jobs::Result<Vec<OutputBlock>> {
        let pattern = prefix_pattern(prefix);
        let mut stmt=self.conn.prepare("SELECT payload FROM recognition_output WHERE session_id=?1 AND generation=?2 AND key LIKE ?3 ESCAPE '!' AND key>?4 ORDER BY key LIMIT 128")?;
        let values = stmt.query_map(params![id, generation, pattern, after], |r| {
            r.get::<_, String>(0)
        })?;
        let mut blocks = Vec::new();
        for value in values {
            let block: OutputBlock = serde_json::from_str(&value?)?;
            block
                .validate()
                .map_err(|e| StoreError::Recognition(e.to_string()))?;
            blocks.push(block);
        }
        Ok(blocks)
    }
    pub fn recognition_last(
        &self,
        id: &str,
        generation: &str,
        prefix: &str,
    ) -> crate::jobs::Result<Option<OutputBlock>> {
        let pattern = prefix_pattern(prefix);
        let value:Option<String>=self.conn.query_row("SELECT payload FROM recognition_output WHERE session_id=?1 AND generation=?2 AND key LIKE ?3 ESCAPE '!' ORDER BY key DESC LIMIT 1",params![id,generation,pattern],|r|r.get(0)).optional()?;
        value
            .map(|s| {
                let block: OutputBlock = serde_json::from_str(&s)?;
                block
                    .validate()
                    .map_err(|e| StoreError::Recognition(e.to_string()))?;
                Ok(block)
            })
            .transpose()
    }
}

impl Store {
    pub fn delete_recognition_prefix(
        &self,
        id: &str,
        generation: &str,
        prefix: &str,
    ) -> crate::jobs::Result<()> {
        delete_prefix(&self.conn, id, generation, prefix)
    }
}

/// The Transcribe checkpoint that names the recognition generation of the
/// note. Its payload is the generation as a JSON string.
pub const GENERATION_KEY: &str = "recognition-generation-v1";

impl Store {
    /// Delete the output rows and the epoch row of one generation. A
    /// replaced generation has no reader.
    pub fn delete_recognition_generation(
        &mut self,
        id: &str,
        generation: &str,
    ) -> crate::jobs::Result<()> {
        let tx = self.conn.transaction()?;
        delete_generation(&tx, id, generation)?;
        tx.commit()?;
        Ok(())
    }

    /// Put a result or speaker index, and delete the blocks under the
    /// `(generation, prefix)` of the index that it replaces, in one
    /// transaction. A fault keeps the old index and its blocks.
    pub fn replace_recognition_index(
        &mut self,
        id: &str,
        stage: crate::Stage,
        key: &str,
        payload: &str,
        replaced: Option<(&str, &str)>,
    ) -> crate::jobs::Result<()> {
        let tx = self.conn.transaction()?;
        crate::jobs::put_checkpoint_in(&tx, id, stage, key, payload)?;
        if let Some((generation, prefix)) = replaced {
            delete_prefix(&tx, id, generation, prefix)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Delete the live NeMo blocks (`nemo/{epoch}/`) of each epoch of the
    /// current generation that has a native-final copy
    /// (`native-final/{epoch}/`). The save of the note calls this.
    /// An epoch that a live worker still writes has no native-final copy,
    /// so its blocks stay.
    pub fn delete_finished_live_blocks(&mut self, id: &str) -> crate::jobs::Result<()> {
        let tx = self.conn.transaction()?;
        let Some(generation) =
            generation_in(&tx, id, &crate::jobs::stage_key(crate::Stage::Transcribe))?
        else {
            return Ok(());
        };
        let epochs = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT substr(key, 14, instr(substr(key, 14), '/') - 1) \
                 FROM recognition_output \
                 WHERE session_id=?1 AND generation=?2 \
                 AND key >= 'native-final/' AND key < 'native-final0' \
                 AND instr(substr(key, 14), '/') > 0",
            )?;
            stmt.query_map(params![id, generation], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for epoch in epochs {
            delete_prefix(&tx, id, &generation, &format!("nemo/{epoch}/"))?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// The generation that the checkpoints of the stage name `stage` hold.
pub(crate) fn generation_in(
    conn: &rusqlite::Connection,
    id: &str,
    stage: &str,
) -> crate::jobs::Result<Option<String>> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload FROM checkpoints WHERE session_id=?1 AND stage=?2 AND key=?3",
            params![id, stage, GENERATION_KEY],
            |r| r.get(0),
        )
        .optional()?;
    Ok(payload.map(|p| serde_json::from_str(&p)).transpose()?)
}

pub(crate) fn delete_generation(
    conn: &rusqlite::Connection,
    id: &str,
    generation: &str,
) -> crate::jobs::Result<()> {
    conn.execute(
        "DELETE FROM recognition_output WHERE session_id=?1 AND generation=?2",
        params![id, generation],
    )?;
    conn.execute(
        "DELETE FROM recognition_epochs WHERE session_id=?1 AND generation=?2",
        params![id, generation],
    )?;
    Ok(())
}

/// Delete the output rows of one generation whose key starts with `prefix`.
/// The key range uses the primary-key index. Keys are ASCII text that the
/// app writes, so the range holds the same rows as a prefix match.
fn delete_prefix(
    conn: &rusqlite::Connection,
    id: &str,
    generation: &str,
    prefix: &str,
) -> crate::jobs::Result<()> {
    let end = format!("{prefix}{}", char::MAX);
    conn.execute(
        "DELETE FROM recognition_output \
         WHERE session_id=?1 AND generation=?2 AND key >= ?3 AND key < ?4",
        params![id, generation, prefix, end],
    )?;
    Ok(())
}

fn prefix_pattern(prefix: &str) -> String {
    format!(
        "{}%",
        prefix
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_")
    )
}
