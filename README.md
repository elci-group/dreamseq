# Dreamseq

End-of-day agent reflection protocol for continuous architectural improvement.

## Overview

Dreamseq is an engineering observability system for human–AI collaboration. Traditional observability tools measure software (latency, errors, throughput); Dreamseq measures the development process itself: where cognition, tooling, model behavior, and workflow introduce friction.

By mining patterns from your AI agent interactions across all harnesses, Dreamseq generates prioritized technical directives that inform the design of the next generation of tools and harnesses.

## Architecture

```
Harnesses (ChatGPT, Kimi, Grok, Claude, OSS, etc.)
        │
        ▼
      Bound
(Log aggregation)
        │
        ▼
Normalization
(remove duplicates, timestamps,
tool calls, provider metadata)
        │
        ▼
Semantic Segmentation
(split into sessions/topics/tasks)
        │
        ▼
GPT OSS 120B (Groq)
Deep reasoning
        │
        ▼
Pattern extraction
        │
        ├── Model failures
        ├── Harness friction
        ├── User steering
        ├── Missing tooling
        ├── Architecture trends
        ├── Workflow bottlenecks
        ├── Repeated shell commands
        ├── Repeated prompts
        ├── Context loss
        └── Automation opportunities
        │
        ▼
Dreamseq Anthology
        │
        ├── SOD Directives
        ├── Tool Candidates
        ├── Agent Improvements
        ├── Prompt Improvements
        ├── Harness Improvements
        ├── New Project Ideas
        └── Priority Scores
```

## Features

- **Log Aggregation**: Collects logs from multiple AI harnesses via Bound integration
- **Normalization**: Removes duplicates, normalizes timestamps, and standardizes metadata
- **Semantic Segmentation**: Groups logs into coherent sessions and topics
- **Deep Analysis**: Uses Groq's `openai/gpt-oss-120b` for pattern recognition and insight extraction
- **User Steering Detection**: Classifies why user intervention was necessary
- **Pattern Extraction**: Identifies recurring issues and automation opportunities
- **Trend Analysis**: Cross-day comparison to identify systemic issues vs one-off annoyances
- **Confidence Scoring**: Quantifies reliability of each directive
- **Kaptaind Integration**: Monitored by kaptaind for automated versioning and commits

## Installation

```bash
# Clone the repository
git clone <repository-url>
cd dreamseq

# Build the project
cargo build --release

# Initialize configuration
cargo run -- init
```

## Configuration

Dreamseq uses a JSON configuration file at `~/.config/dreamseq/config.json` (legacy TOML files are also accepted):

```json
{
  "groq_api_key": "your-groq-api-key",
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
    }
  ],
  "output_dir": "/home/user/dreamseq/output",
  "anthologies_dir": "/home/user/dreamseq/anthologies",
  "enable_tts": false,
  "enable_kaptaind": true
}
```

### Configuration Options

- `groq_api_key`: API key for Groq (required for analysis)
- `harnesses`: Array of harness configurations
  - `name`: Identifier for the harness
  - `log_path`: Path to log files
  - `log_format`: Format of logs (`json`, `markdown`, `plain`, or `custom`)
- `output_dir`: Directory for temporary output files
- `anthologies_dir`: Directory for generated anthologies
- `enable_tts`: Enable text-to-speech notifications
- `enable_kaptaind`: Enable kaptaind monitoring integration

## Usage

### Initialize Configuration

```bash
dreamseq init
```

### Run Analysis

```bash
# Run with default configuration
dreamseq run

# Run with custom configuration
dreamseq run --config /path/to/config.json
```

### View Reports

```bash
# View anthology for a specific date
dreamseq report 2026-08-07

# View trend analysis
dreamseq trends --days 30
```

## Output Structure

Dreamseq generates deterministic anthologies with the following structure:

### Executive Summary
High-level overview of findings and impact.

### Significant Milestones
Notable events and high-impact patterns identified.

### User Behaviour
- Repeated git workflows
- Repeated package installations
- Repeated file navigation patterns
- Other behavioral patterns

### Model Weaknesses
Per-model analysis of recurring issues and failures.

### Harness Weaknesses
Per-harness friction points and severity scores.

### Candidate Tools
Prioritized tool suggestions with:
- Priority level (High/Medium/Low)
- Reason for suggestion
- Estimated time saved
- Confidence score
- Affected projects

### Steering Events
Classification of user interventions:
- Missing tool
- Missing context
- Wrong abstraction
- Excess verbosity
- Hallucination
- Architectural mismatch
- Manual repetition

### Trend Analysis
Cross-day comparison showing:
- Current vs previous values
- Trend direction (increasing/decreasing/stable)
- ASCII visualizations

## Directive Format

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

## Kaptaind Integration

Dreamseq is monitored by kaptaind for:
- Automated semantic versioning
- Deterministic git commits
- Change analysis
- Visual status feedback

When kaptaind is enabled, Dreamseq will:
- Check kaptaind status before running
- Run kaptaind analysis after completion
- Leverage kaptaind's commit automation for anthology updates

## Development

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

## Roadmap

### Recently completed
- Source-control hygiene (`.gitignore`, initial commit, ignored runtime artifacts).
- Added MIT `LICENSE` and GitHub Actions CI workflow.
- Corrected normalization duplicate/empty entry accounting.
- Steering detector now compiles regexes once and uses tighter patterns to avoid false positives on telemetry noise.
- Expanded test coverage from 13 to 25 tests, replacing tautological assertions with real checks and adding a mocked Groq endpoint test.
- Removed unused `config` and `thiserror` dependencies.
- Improved log parsing:
  - JSON logs now extract `tool_calls`, `model`, and numeric or string timestamps.
  - Plain and Markdown parsers extract inline timestamps and skip headings/empty lines.
  - Codex SQLite sources are skipped gracefully when `sqlite3` is unavailable.

### Up next
- Replace heuristic segmentation (Jaccard + time gap) with an embedding-based or topic-model approach.
- Add real Bound integration rather than direct file scanning.
- Add harness-specific structured parsers (e.g., OpenAI Codex JSONL, Kimi session logs).
- Add deterministic end-to-end integration tests with fixture data.

## License

This project is licensed under the [MIT License](LICENSE).

## Contributing

Contributions are welcome. Please open an issue or pull request to discuss non-trivial changes before submitting code.

## Acknowledgments

Built with:
- Rust
- Groq API
- Kaptaind
- Bound (log aggregation)
