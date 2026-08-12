# Dreamsequence

> Dreamsequence — Engineering intelligence, in motion.

Dreamseq turns recurring friction in agent-native engineering work into verified capabilities.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/lockup-white.png">
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/lockup-black.png">
    <img alt="Dreamsequence" src="docs/assets/lockup-black.png" width="560">
  </picture>
</p>

Individual lockups, symbols, and application icons are available in [`docs/assets`](docs/assets/) with usage guidance in the [brand documentation](docs/brand.md).

![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)
![License: MIT](https://img.shields.io/badge/license-MIT-green.svg?style=for-the-badge)

Dreamseq is an engineering observability system for human–AI collaboration. Traditional observability tools measure software (latency, errors, throughput); Dreamseq measures the **development process itself**: where cognition, tooling, model behavior, and workflow introduce friction.

By mining patterns from your AI agent interactions across all harnesses, Dreamseq generates prioritized technical directives that inform the design of the next generation of tools and harnesses.

---

## 🏗️ Architecture

```text
🖥️  Harnesses (ChatGPT, Kimi, Grok, Claude, OSS, etc.)
           │
           ▼
      ┌─────────┐
      │  Bound  │  📦 Log aggregation
      └────┬────┘
           │
           ▼
   ┌───────────────┐
   │ Normalization │  🧹 Retain repetitions; normalize timestamps,
   └───────┬───────┘     tool calls, provider metadata
           │
           ▼
   ┌──────────────────┐
   │ Semantic Segm.   │  ✂️ Split into sessions / topics / tasks
   └────────┬─────────┘
            │
            ▼
   ┌──────────────────┐
   │ GPT OSS 120B     │  🧠 Deep reasoning (via Groq)
   │   (Groq)         │
   └────────┬─────────┘
            │
            ▼
   ┌──────────────────┐
   │ Pattern Extraction│
   └────────┬─────────┘
            │
            ├── 🤖 Model failures
            ├── 🪝 Harness friction
            ├── 🎯 User steering
            ├── 🔧 Missing tooling
            ├── 📐 Architecture trends
            ├── ⏱️ Workflow bottlenecks
            ├── 🔁 Repeated shell commands
            ├── 🔁 Repeated prompts
            ├── 🌫️ Context loss
            └── ⚡ Automation opportunities
            │
            ▼
   ┌──────────────────┐
   │ Dreamseq Anthology│
   └────────┬─────────┘
            │
            ├── 📜 SOD Directives
            ├── 🛠️ Tool Candidates
            ├── 🤖 Agent Improvements
            ├── 📝 Prompt Improvements
            ├── 🪝 Harness Improvements
            ├── 💡 New Project Ideas
            └── 📊 Priority Scores
```

---

## ⚡ Features

| Feature | Icon | What it does |
|---|---|---|
| Log Aggregation | 📥 | Collects logs from multiple AI harnesses via Bound integration. |
| Normalization | 🧹 | Standardizes content and metadata while retaining repeated events as frequency evidence. |
| Semantic Segmentation | ✂️ | Groups logs into coherent sessions and topics. |
| Deep Analysis | 🧠 | Uses Groq's `openai/gpt-oss-120b` for pattern recognition and insight extraction. |
| User Steering Detection | 🎯 | Classifies why user intervention was necessary. |
| Pattern Extraction | 🔍 | Identifies recurring issues and automation opportunities. |
| Trend Analysis | 📈 | Cross-day comparison to identify systemic issues vs one-off annoyances. |
| Heuristic Scoring | 📊 | Ranks directives using bounded frequency, severity, and model-confidence signals; scores are prioritization aids rather than calibrated probabilities. |
| Kaptaind Integration | 🤖 | Monitored by kaptaind for automated versioning and commits. |

---

## 🚀 Installation

```bash
# Clone the repository
git clone <repository-url>
cd dreamseq

# Build the project
cargo build --release

# Initialize configuration
cargo run -- init
```

### Pair with Dreamsequence.pro

Pairing uses a short-lived browser code. The resulting device credential is stored at `~/.config/dreamseq/credentials.json` with owner-only permissions; raw prompts and log files are never uploaded.

```bash
# Open the browser pairing page and approve this device
dreamseq login

# Successful runs sync a privacy-reduced summary automatically
dreamseq run

# Upload existing anthology JSON from configured output directories
dreamseq sync

# Or scan explicit directories
dreamseq sync --dir ~/dreamseq/anthologies --dir /path/to/other/output

# Revoke the server token before removing the local credential
dreamseq logout
```

For self-hosted development, pass `--api-url https://your-host` to `login`. Plain HTTP is accepted only for loopback test servers.

### Inference routing and BYOK fallback

When a device is paired, Dreamseq sends each already-redacted analysis batch to the Dreamsequence production API first. Existing credentials for the legacy `dreamsequence.pro` origin are migrated in memory to the current TLS endpoint, so users do not need to re-pair. Retryable cloud failures, unavailable service configuration, and invalid model output fall through to local BYOK routes in order. Set `DREAMSEQUENCE_API_URL` for an approved self-hosted endpoint or `DREAMSEQ_DISABLE_CLOUD_INFERENCE=1` to use BYOK exclusively.

The legacy `GROQ_API_KEY` remains an automatic final route. Any OpenAI-compatible providers can be configured without writing their keys into Dreamseq files:

```bash
export PRIMARY_INFERENCE_KEY='...'
export SECONDARY_INFERENCE_KEY='...'
export DREAMSEQ_BYOK_ROUTES='[
  {"name":"primary","base_url":"https://primary.example/v1","model":"provider-model","api_key_env":"PRIMARY_INFERENCE_KEY"},
  {"name":"secondary","base_url":"https://secondary.example/v1","model":"fallback-model","api_key_env":"SECONDARY_INFERENCE_KEY"}
]'
```

For a single provider, set `DREAMSEQ_BYOK_API_KEY`, `DREAMSEQ_BYOK_BASE_URL`, and `DREAMSEQ_BYOK_MODEL` together. Non-loopback provider URLs must use HTTPS.

---

## ⚙️ Configuration

Dreamseq uses a JSON configuration file at `~/.config/dreamseq/config.json` (legacy TOML files are also accepted):

```json
{
  "harnesses": [
    {
      "name": "chatgpt",
      "log_path": "/path/to/chatgpt/logs",
      "log_format": "json"
    },
    {
      "name": "claude",
      "log_path": "/path/to/claude/logs",
      "log_format": "markdown"
    },
    {
      "name": "project-snapshots",
      "log_path": "/path/to/project",
      "log_format": "bound",
      "bound_filter": "[.rs]"
    }
  ],
  "output_dir": "/home/user/dreamseq/output",
  "anthologies_dir": "/home/user/dreamseq/anthologies",
  "enable_tts": false,
  "enable_kaptaind": false,
  "allow_remote_analysis": false
}
```

### Configuration Options

| Option | Required? | Description |
|---|---|---|
| `GROQ_API_KEY` environment variable | Optional fallback | Legacy Groq BYOK credential; never written by Dreamseq. |
| `DREAMSEQ_BYOK_ROUTES` environment variable | ❌ No | Ordered JSON array of OpenAI-compatible fallback routes. Each route names the environment variable containing its key. |
| `DREAMSEQ_BYOK_API_KEY`, `DREAMSEQ_BYOK_BASE_URL`, `DREAMSEQ_BYOK_MODEL` | ❌ No | Shorthand for one BYOK fallback route. Set all three together. |
| `DREAMSEQ_DISABLE_CLOUD_INFERENCE` | ❌ No | Set to `1` to bypass paired Dreamsequence inference and use BYOK only. |
| `harnesses` | ✅ Yes | Array of harness configurations. |
| `harnesses[].name` | ✅ Yes | Identifier for the harness. |
| `harnesses[].log_path` | ✅ Yes | Path to log files or project snapshots. |
| `harnesses[].log_format` | ✅ Yes | Format of logs: `json`, `markdown`, `plain`, `codex_sqlite`, or `bound`. |
| `harnesses[].bound_filter` | ❌ No | Optional Bound filter (e.g. `[.rs]`) when `log_format` is `bound`. |
| `output_dir` | ✅ Yes | Directory for temporary output files. |
| `anthologies_dir` | ✅ Yes | Directory for generated anthologies. |
| `enable_tts` | ✅ Yes | Enable text-to-speech notifications. |
| `enable_kaptaind` | ✅ Yes | Opt in to Kaptaind status and analysis integration. Defaults to `false` because external analyzers may write project metadata. |
| `allow_remote_analysis` | ✅ Yes | Explicit consent to send redacted excerpts to Dreamsequence or configured BYOK endpoints. Defaults to `false`. |

---

## 🎮 Usage

### Initialize Configuration

```bash
dreamseq init
```

### Run Analysis

```bash
# Run with default configuration after setting `allow_remote_analysis: true`
dreamseq run

# Run with custom configuration
dreamseq run --config /path/to/config.json

# Explicitly publish .dreams backlogs to Speck-registered projects
dreamseq run --publish-dreams

# Machine-readable completion output
dreamseq run --json
```

### View Reports

```bash
# View anthology for a specific date
dreamseq report 2026-08-07

# Report is read-only unless publishing is explicitly requested
dreamseq report 2026-08-07 --json
dreamseq report 2026-08-07 --publish-dreams

# Report or trend against a custom anthology directory
dreamseq report 2026-08-07 --config /path/to/config.json
dreamseq trends --days 30 --config /path/to/config.json

# View trend analysis
dreamseq trends --days 30
```

---

## 📁 Output Structure

Dreamseq generates versioned anthologies with stable schemas. Each run has a unique ID and timestamp, so multiple runs on the same day are retained rather than overwritten.

Each run also writes a structured ingestion report under `output_dir`. It records files seen, files rejected, accepted-entry counts, and contextual warnings.

### Privacy boundary

Remote analysis is opt-in. When enabled, Dreamseq sends bounded batches of log excerpts to Groq, redacts common credential assignments and JWTs, treats excerpts as untrusted prompt data, and retries transient API failures. Redaction is defense in depth rather than a guarantee: review configured sources before enabling remote analysis, especially when using Bound on source trees.

### 📋 Executive Summary
High-level overview of findings and impact.

### 🏆 Significant Milestones
Notable events and high-impact patterns identified.

### 👤 User Behaviour
- 🔁 Repeated git workflows
- 📦 Repeated package installations
- 📂 Repeated file navigation patterns
- 🧩 Other behavioral patterns

### 🤖 Model Weaknesses
Per-model analysis of recurring issues and failures.

### 🪝 Harness Weaknesses
Per-harness friction points and severity scores.

### 🛠️ Candidate Tools
Prioritized tool suggestions with:
- Priority level (🔴 High / 🟡 Medium / 🟢 Low)
- Reason for suggestion
- Estimated time saved
- Confidence score
- Affected projects

### 🎯 Steering Events
Classification of user interventions:

| Category | What it means |
|---|---|
| Missing tool | The agent lacked a tool to do the job. |
| Missing context | The agent lost or never had enough context. |
| Wrong abstraction | The solution was built at the wrong level of abstraction. |
| Excess verbosity | The response was too long or noisy. |
| Hallucination | The model invented facts or APIs. |
| Architectural mismatch | The proposal conflicted with the project architecture. |
| Manual repetition | The user had to repeat the same action manually. |

### 📈 Trend Analysis
Cross-day comparison showing:
- Current vs previous values
- Trend direction (📈 increasing / 📉 decreasing / ➡️ stable)
- ASCII visualizations

---

## 📜 Directive Format

Each directive includes:

```yaml
directive:
  id: DS-1043
  title: Create daemon for package installation
  frequency: 31
  estimated_time_saved: 47 min/week
  confidence: 0.94
  automation_score: 0.98
  implementation_effort: medium
  affected_projects:
    - baby
    - goblin
    - workhorse
```

---

## 🤖 Kaptaind Integration

Dreamseq is monitored by kaptaind for:
- 🏷️ Automated semantic versioning
- 📝 Deterministic git commits
- 🔍 Change analysis
- 🚦 Visual status feedback

When kaptaind is enabled, Dreamseq will:
- Check kaptaind status before running.
- Run kaptaind analysis after completion.
- Leverage kaptaind's commit automation for anthology updates.

---

## 🧑‍💻 Development

### Running Tests

```bash
cargo test
```

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release
```

### Code Quality

```bash
# Check for errors
cargo check

# Format code
cargo fmt

# Run clippy
cargo clippy
```

---

## 🗺️ Roadmap

### ✅ Recently completed
| Milestone | Notes |
|---|---|
| Repository hygiene | `.gitignore`, initial commit, ignored runtime artifacts. |
| Legal & CI | Added MIT `LICENSE` and GitHub Actions CI workflow. |
| Normalization fix | Preserved repeated-event evidence while removing empty entries. |
| Steering detector | Compiles regexes once and uses tighter patterns to avoid false positives on telemetry noise. |
| Test coverage | Expanded from 13 to 31 tests, replacing tautological assertions with real checks and adding a mocked Groq endpoint test. |
| Integration test | Added deterministic end-to-end integration test using fixture log data and a local mock Groq server. |
| Dependency cleanup | Removed unused `config` and `thiserror` dependencies. |
| Log parsing improvements | JSON logs now extract `tool_calls`, `model`, and numeric or string timestamps; harness-specific field fallbacks (`ts`, `text`, `body`, `msg`); plain/Markdown timestamp extraction; graceful Codex SQLite skip. |
| Segmentation | Topic similarity now uses TF-IDF weighted cosine similarity over stopword-filtered tokens. |
| Pipeline test | End-to-end pipeline test runs without a Groq API key when no logs are present. |
| Bound integration | Integrated [Bound](https://github.com/sal/bound) as a first-class log source via the `bound` `log_format` and optional `bound_filter`. |

### 📌 Up next
| Idea | Goal |
|---|---|
| Embedding-based segmentation | Richer semantic grouping beyond TF-IDF cosine. |
| Dedicated harness parsers | Handle proprietary or binary formats that generic parsers cannot manage. |

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).

---

## 🤝 Contributing

Contributions are welcome. Please open an issue or pull request to discuss non-trivial changes before submitting code.

---

## 🙏 Acknowledgments

Built with:
- 🦀 Rust
- ⚡ Groq API
- 🤖 Kaptaind
- 📦 Bound (log aggregation)
