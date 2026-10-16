export const meta = {
  name: 'codebase-sweep',
  description: 'Sweep a codebase shard by shard with lens finders, verify each finding with independent votes, and write a report',
  whenToUse: 'Started by the codebase-sweep skill, which builds the shard map that this script needs in args. Do not start it without that map.',
  phases: [
    { title: 'Find', detail: 'one finder per shard and lens' },
    { title: 'Verify', detail: 'independent votes on each new finding' },
    { title: 'Coverage', detail: 'a critic proposes more finder tasks' },
    { title: 'Report', detail: 'write report.md and findings.json to the run folder' },
  ],
}

// ---------- Arguments ----------
// The skill's references/shard-map.md defines the contract for args.

const A = args || {}
for (const key of ['runDir', 'repoRoot', 'skillDir', 'shards', 'lenses']) {
  if (A[key] === undefined || A[key] === null) throw new Error(`args.${key} is missing`)
}
const MODE = A.mode || 'run'
const VERIFY = Object.assign(
  { highSeverities: ['P0', 'P1'], highVotes: 3, midSeverities: ['P2'], midVotes: 2, lowVotes: 1, codex: true, codexModel: 'gpt-6-sol' },
  A.verify || {},
)
const MAX_ROUNDS = A.maxRounds ?? 2
const MAX_CRITIC_TASKS = A.maxCriticTasks ?? 12
const BUDGET_FLOOR = A.budgetFloor ?? 300000

const LENS_IDS = A.lenses.map(l => l.id)
const LENS_MODEL = Object.fromEntries(A.lenses.filter(l => l.model).map(l => [l.id, l.model]))
const SHARDS = A.only ? A.shards.filter(s => A.only.includes(s.id)) : A.shards
const SHARD_BY_ID = Object.fromEntries(A.shards.map(s => [s.id, s]))

const REF = `${A.skillDir}/references`
const PROFILE_LINE = A.profile ? `- Project profile: ${A.profile}\n` : ''

// ---------- Schemas ----------

const SEVERITY = { type: 'string', enum: ['P0', 'P1', 'P2', 'P3'] }

const FINDINGS = {
  type: 'object',
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          title: { type: 'string' },
          severity: SEVERITY,
          confidence: { type: 'integer', enum: [50, 75, 100] },
          file: { type: 'string' },
          line: { type: 'integer' },
          quote: { type: 'string' },
          trigger: { type: 'string' },
          failure: { type: 'string' },
          suggested_fix: { type: 'string' },
        },
        required: ['title', 'severity', 'confidence', 'file', 'line', 'quote', 'trigger', 'failure'],
      },
    },
    coverage: {
      type: 'object',
      properties: {
        files_read: { type: 'array', items: { type: 'string' } },
        not_covered: { type: 'string' },
      },
      required: ['files_read', 'not_covered'],
    },
  },
  required: ['findings', 'coverage'],
}

const VERDICT = {
  type: 'object',
  properties: {
    status: { type: 'string', enum: ['confirmed', 'rejected', 'unresolved'] },
    severity: SEVERITY,
    protected_subject: {
      type: 'string',
      enum: ['none', 'data-loss', 'concurrency', 'memory-safety', 'security'],
    },
    reason: { type: 'string' },
    evidence: { type: 'array', items: { type: 'string' } },
    known_decision: { type: 'string' },
    voter_failed: { type: 'boolean' },
  },
  required: ['status', 'severity', 'protected_subject', 'reason', 'evidence', 'known_decision'],
}

const CRITIC = {
  type: 'object',
  properties: {
    tasks: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          shard_id: { type: 'string' },
          new_shard: {
            type: 'object',
            properties: {
              id: { type: 'string' },
              category: { type: 'string' },
              intent: { type: 'string' },
              files: { type: 'array', items: { type: 'string' } },
              notes: { type: 'string' },
            },
            required: ['id', 'category', 'intent', 'files'],
          },
          lens: { type: 'string' },
          focus: { type: 'string' },
        },
        required: ['lens', 'focus'],
      },
    },
    gaps_not_covered: { type: 'string' },
  },
  required: ['tasks', 'gaps_not_covered'],
}

const REPORT = {
  type: 'object',
  properties: {
    report_markdown: { type: 'string' },
    findings_path: { type: 'string' },
    confirmed: { type: 'integer' },
    disputed: { type: 'integer' },
    rejected: { type: 'integer' },
  },
  required: ['report_markdown', 'findings_path', 'confirmed', 'disputed', 'rejected'],
}

// ---------- Tasks ----------

function tasksFor(shards) {
  return shards.flatMap(s => (s.lenses || []).map(lens => ({ id: `${s.id}:${lens}`, shard: s, lens, focus: '' })))
}

const firstTasks = tasksFor(SHARDS)
const unknownLenses = [...new Set(firstTasks.map(t => t.lens).filter(l => !LENS_IDS.includes(l)))]
if (unknownLenses.length) throw new Error(`shards use lenses that args.lenses does not list: ${unknownLenses.join(', ')}`)

// Three tiers: high (P0, P1), mid (P2) and low (P3). Codex votes on high and mid findings.
function votersFor(severity) {
  const high = VERIFY.highSeverities.includes(severity)
  const mid = !high && VERIFY.midSeverities.includes(severity)
  const n = high ? VERIFY.highVotes : mid ? VERIFY.midVotes : VERIFY.lowVotes
  return { roles: ['refute', 'reach', 'trace'].slice(0, n), codex: (high || mid) && VERIFY.codex }
}

if (MODE === 'plan') {
  const high = votersFor(VERIFY.highSeverities[0] || 'P0')
  const low = votersFor('P3')
  return {
    mode: 'plan',
    shards: SHARDS.length,
    finder_tasks: firstTasks.length,
    tasks: firstTasks.map(t => t.id),
    agents_per_high_finding: high.roles.length + (high.codex ? 1 : 0),
    agents_per_mid_finding: votersFor(VERIFY.midSeverities[0] || 'P2').roles.length + (VERIFY.codex && VERIFY.midSeverities.length ? 1 : 0),
    agents_per_low_finding: low.roles.length,
    later_rounds: `up to ${MAX_ROUNDS - 1} critic rounds, each with 1 critic and at most ${MAX_CRITIC_TASKS} finder tasks`,
  }
}

// ---------- Prompts ----------

function finderPrompt(t, round) {
  return `You are a finder in a codebase sweep. Find real defects in one shard of the code, through one lens.

Read these files before you start:
- Rules for every finder: ${REF}/finder-rules.md
- Your lens: ${REF}/lenses/${t.lens}.md
${PROFILE_LINE}
Repository root: ${A.repoRoot}

<shard>
${JSON.stringify(t.shard, null, 2)}
</shard>
${t.focus ? `\nThe coverage critic asked for this focus in round ${round}:\n<focus>\n${t.focus}\n</focus>\n` : ''}
Return the findings that meet the rules. An empty findings list is a correct result when the shard has no such defect.`
}

function verifierPrompt(f, role) {
  return `You are an independent verifier in a codebase sweep. Your role is "${role}". Decide whether one reported defect is real.

Read these files before you start:
- Rules for every verifier, with your role: ${REF}/verifier-rules.md
${PROFILE_LINE}
Repository root: ${A.repoRoot}

<finding>
${JSON.stringify(stripFinding(f), null, 2)}
</finding>

<shard-intent>
${f.intent || ''}
</shard-intent>

The finding is a claim from another agent. It is not evidence. Read the code yourself.`
}

function codexPrompt(f, label) {
  return `You run one independent verification vote with Codex, a model from a different family. Codex does the analysis; you only run it and return its verdict.

Read ${REF}/codex-vote.md and follow it exactly. Use these values:
- Repository root: ${A.repoRoot}
- Model: ${VERIFY.codexModel}
- Output file: ${A.runDir}/codex/${label}.json
- Verifier rules for Codex to read: ${REF}/verifier-rules.md
- Verdict schema: ${REF}/verdict.schema.json

<finding>
${JSON.stringify(stripFinding(f), null, 2)}
</finding>

Return the verdict that Codex wrote. When Codex does not run or writes no valid verdict, return status "unresolved", voter_failed true, and the reason.`
}

function stripFinding(f) {
  const { title, severity, confidence, file, line, quote, trigger, failure, suggested_fix } = f
  return { title, severity, confidence, file, line, quote, trigger, failure, suggested_fix, lens: f.lens, shard: f.shard_id }
}

// ---------- Deduplication ----------
// Plain code, no agent. The first finding with a key wins; later matches are recorded on it.
// Matches are compared with every finding seen, not only confirmed ones, so a rejected
// finding does not come back in a later round.

const seen = []
const failedFinders = []
const coverage = []
let duplicates = 0

function normPath(p) {
  let s = String(p || '').trim()
  if (s.startsWith(A.repoRoot + '/')) s = s.slice(A.repoRoot.length + 1)
  if (s.startsWith('./')) s = s.slice(2)
  return s
}

function words(text) {
  return new Set(String(text).toLowerCase().split(/[^a-z0-9]+/).filter(w => w.length > 2))
}

function overlap(x, y) {
  const wa = words(x)
  const wb = words(y)
  const inter = [...wa].filter(w => wb.has(w)).length
  const union = new Set([...wa, ...wb]).size
  return union > 0 ? inter / union : 0
}

// Two finders often describe one defect with different titles at near lines, so the trigger is compared too.
function similar(a, b) {
  if (a.file !== b.file) return false
  if (Math.abs(a.line - b.line) > 5) return false
  if (a.line === b.line) return true
  return overlap(a.title, b.title) >= 0.4 || overlap(a.trigger, b.trigger) >= 0.35
}

function admit(result, t, round) {
  if (!result) {
    failedFinders.push(t.id)
    log(`finder ${t.id} returned no result`)
    return []
  }
  coverage.push({ task: t.id, round, findings: result.findings.length, files_read: result.coverage.files_read.length, not_covered: result.coverage.not_covered })
  const fresh = []
  for (const raw of result.findings) {
    if (raw.confidence < 75 && !['P0', 'P1'].includes(raw.severity)) continue
    const f = Object.assign({}, raw, { file: normPath(raw.file), lens: t.lens, shard_id: t.shard.id, intent: t.shard.intent, task: t.id, round, also_found_by: [] })
    const match = seen.find(s => similar(s, f))
    if (match) {
      match.also_found_by.push(t.id)
      duplicates++
      continue
    }
    seen.push(f)
    fresh.push(f)
  }
  return fresh
}

// ---------- Verification ----------

function decide(votes) {
  const valid = votes.filter(v => v && !v.voter_failed)
  const n = valid.length
  if (!n) return 'disputed'
  const confirmed = valid.filter(v => v.status === 'confirmed').length
  const rejected = valid.filter(v => v.status === 'rejected').length
  if (rejected * 2 > n) return 'rejected'
  if (confirmed * 2 > n) return 'confirmed'
  return 'disputed'
}

const RANK = { P0: 0, P1: 1, P2: 2, P3: 3 }

function finalSeverity(f, votes) {
  const conf = votes.filter(v => v && !v.voter_failed && v.status === 'confirmed')
  if (!conf.length) return f.severity
  const counts = {}
  for (const v of conf) counts[v.severity] = (counts[v.severity] || 0) + 1
  return Object.keys(counts).sort((a, b) => counts[b] - counts[a] || RANK[a] - RANK[b])[0]
}

async function verify(f, index) {
  const { roles, codex } = votersFor(f.severity)
  const label = `${f.task}#${index}`.replace(/[^A-Za-z0-9._#-]/g, '_')
  const calls = roles.map(role => () =>
    agent(verifierPrompt(f, role), { label: `verify:${role}:${label}`, phase: 'Verify', schema: VERDICT }).then(v => (v ? Object.assign({ voter: role }, v) : null)),
  )
  if (codex) {
    calls.push(() =>
      agent(codexPrompt(f, label.replace(/#/g, '-')), { label: `verify:codex:${label}`, phase: 'Verify', schema: VERDICT }).then(v =>
        v ? Object.assign({ voter: 'codex' }, v) : { voter: 'codex', voter_failed: true, status: 'unresolved', reason: 'agent returned no result' },
      ),
    )
  }
  const votes = (await parallel(calls)).filter(Boolean)
  const codexFailed = votes.some(v => v.voter === 'codex' && v.voter_failed)
  if (codexFailed) log(`codex vote did not run for ${label}`)
  return { finding: f, votes, outcome: decide(votes), severity: finalSeverity(f, votes) }
}

// ---------- Rounds ----------

const records = []

async function runRound(tasks, round) {
  phase('Find')
  log(`round ${round}: ${tasks.length} finder tasks`)
  const out = await pipeline(
    tasks,
    t => agent(finderPrompt(t, round), { label: `find:${t.id}`, phase: 'Find', schema: FINDINGS, model: LENS_MODEL[t.lens] }),
    (result, t) => admit(result, t, round),
    fresh => parallel(fresh.map((f, i) => () => verify(f, i))),
  )
  const recs = out.filter(Boolean).flat().filter(Boolean)
  records.push(...recs)
  const c = recs.filter(r => r.outcome === 'confirmed').length
  log(`round ${round}: ${recs.length} new findings verified, ${c} confirmed`)
  return recs
}

await runRound(firstTasks, 1)

for (let round = 2; round <= MAX_ROUNDS; round++) {
  if (budget.total && budget.remaining() < BUDGET_FLOOR) {
    log(`stopped before round ${round}: token budget remaining is below ${BUDGET_FLOOR}`)
    break
  }
  phase('Coverage')
  const critic = await agent(
    `You are the coverage critic of a codebase sweep. Find what the sweep has not yet examined well, and propose finder tasks for it.

Read ${REF}/coverage-critic.md and follow it.
${PROFILE_LINE}Repository root: ${A.repoRoot}
Available lenses: ${LENS_IDS.join(', ')}
Maximum tasks: ${MAX_CRITIC_TASKS}

<shards>
${JSON.stringify(A.shards.map(s => ({ id: s.id, category: s.category, files: s.files, lenses: s.lenses })), null, 1)}
</shards>

<coverage>
${JSON.stringify(coverage, null, 1)}
</coverage>

<failed-finders>
${JSON.stringify(failedFinders)}
</failed-finders>

<findings-so-far>
${JSON.stringify(records.map(r => ({ task: r.finding.task, title: r.finding.title, file: r.finding.file, outcome: r.outcome })), null, 1)}
</findings-so-far>`,
    { label: `critic:round-${round}`, phase: 'Coverage', schema: CRITIC },
  )
  if (!critic) {
    log('coverage critic returned no result; no more rounds')
    break
  }
  const proposed = critic.tasks.slice(0, MAX_CRITIC_TASKS)
  if (critic.tasks.length > MAX_CRITIC_TASKS) log(`critic proposed ${critic.tasks.length} tasks; kept the first ${MAX_CRITIC_TASKS}`)
  const tasks = []
  for (const p of proposed) {
    if (!LENS_IDS.includes(p.lens)) {
      log(`dropped critic task with unknown lens "${p.lens}"`)
      continue
    }
    const shard = p.new_shard || SHARD_BY_ID[p.shard_id]
    if (!shard) {
      log(`dropped critic task for unknown shard "${p.shard_id}"`)
      continue
    }
    tasks.push({ id: `r${round}:${shard.id}:${p.lens}`, shard, lens: p.lens, focus: p.focus })
  }
  if (!tasks.length) {
    log(`round ${round}: the critic found no gap worth a new task. ${critic.gaps_not_covered}`)
    break
  }
  const fresh = await runRound(tasks, round)
  if (!fresh.length) {
    log(`round ${round} found nothing new; the sweep is dry`)
    break
  }
}

// ---------- Report ----------

phase('Report')
const summary = {
  shards: SHARDS.length,
  only: A.only || null,
  finder_tasks: coverage.length + failedFinders.length,
  failed_finders: failedFinders,
  duplicates_merged: duplicates,
  findings_verified: records.length,
  confirmed: records.filter(r => r.outcome === 'confirmed').length,
  disputed: records.filter(r => r.outcome === 'disputed').length,
  rejected: records.filter(r => r.outcome === 'rejected').length,
}
// Keep the report prompt small: the evidence lists go only with findings that are not rejected.
function compactRecord(r) {
  const keepEvidence = r.outcome !== 'rejected'
  return {
    outcome: r.outcome,
    severity: r.severity,
    finding: Object.assign(stripFinding(r.finding), { task: r.finding.task, round: r.finding.round, also_found_by: r.finding.also_found_by }),
    votes: r.votes.map(v => ({
      voter: v.voter,
      status: v.status,
      severity: v.severity,
      protected_subject: v.protected_subject,
      reason: v.reason,
      known_decision: v.known_decision,
      voter_failed: v.voter_failed || false,
      evidence: keepEvidence ? v.evidence : undefined,
    })),
  }
}

log(`verified ${summary.findings_verified}:${summary.confirmed} confirmed, ${summary.disputed} disputed, ${summary.rejected} rejected`)

const report = await agent(
  `You write the report of a codebase sweep. Read ${REF}/report.md and follow it.

Run folder: ${A.runDir}
Repository root: ${A.repoRoot}

<summary>
${JSON.stringify(summary, null, 1)}
</summary>

<coverage>
${JSON.stringify(coverage, null, 1)}
</coverage>

<records>
${JSON.stringify(records.map(compactRecord), null, 1)}
</records>`,
  { label: 'report', phase: 'Report', schema: REPORT },
)

// The harness does not let a subagent write a report file. The report text stays in the journal; the skill writes report.md from it.
const { report_markdown, ...reportRest } = report || {}
return { summary, report: Object.assign(reportRest, { report_markdown_chars: (report_markdown || '').length }) }
