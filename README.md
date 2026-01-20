# Ralph

[![CI](https://github.com/kcirtapfromspace/ralph-machineo/actions/workflows/ci.yml/badge.svg)](https://github.com/kcirtapfromspace/ralph-machineo/actions/workflows/ci.yml)
[![Docker](https://github.com/kcirtapfromspace/ralph-machineo/actions/workflows/docker.yml/badge.svg)](https://github.com/kcirtapfromspace/ralph-machineo/actions/workflows/docker.yml)

![Ralph](ralph-machineo.webp)

Ralph is an autonomous AI agent loop that runs [Claude Code](https://docs.anthropic.com/en/docs/claude-code), Codex, Amp, or local llms repeatedly until all PRD items are complete. Each iteration is a fresh agent instance with clean context. Memory persists via git history, `progress.txt`, and `prd.json`. Ralph can also run as an MCP server and execute stories in parallel batches.

Based on [Geoffrey Huntley's Ralph pattern](https://ghuntley.com/ralph/).
[Read Ryan Carson in-depth article on how he use Ralph](https://x.com/ryancarson/status/2008548371712135632)
Orignally forked from [snarktank/ralph](https://github.com/snarktank/ralph)

## Prerequisites

- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code), Codex CLI, or Amp CLI in PATH
- Optional: local Ollama server for OSS models (e.g. `~/off-quant` with `tilt up`)
- [Rust](https://rustup.rs) (for building from source)
- A git repository for your project

## Installation

### Homebrew (macOS/Linux)

```bash
brew tap kcirtapfromspace/ralph
brew install ralph
```

Or in one command:

```bash
brew install kcirtapfromspace/ralph/ralph
```

### Build from Source

Requires [Rust](https://rustup.rs) to be installed.

```bash
git clone https://github.com/kcirtapfromspace/ralph-machineo.git ~/.ralph
cd ~/.ralph
./install.sh
```

This builds the Rust binary and installs it to `/usr/local/bin`. For a different location:

```bash
./install.sh ~/bin        # Install to ~/bin
sudo ./install.sh         # If you need sudo for /usr/local/bin
```

Then add to your shell config (`~/.bashrc`, `~/.zshrc`):

```bash
export RALPH_HOME="$HOME/.ralph"
```

To uninstall:
```bash
./uninstall.sh
```

### Docker / Claude Desktop (MCP)

Prefer Claude Desktop over the terminal? Ralph is also available as an MCP server via Docker:

```bash
docker pull ghcr.io/kcirtapfromspace/ralph-machineo:latest
```

See the [Docker MCP Setup Guide](docs/guides/docker-mcp-setup.md) for Claude Desktop configuration.

## Usage

```bash
ralph [OPTIONS] [MAX_ITERATIONS]
ralph <COMMAND>

Commands:
  init    Initialize project with prd.json template
  home    Show Ralph installation directory

Options:
  -d, --dir <PATH>       Working directory (default: current directory)
  -p, --prompt <FILE>    Custom prompt file
  -n, --iterations <N>   Max iterations (default: 10)
  --agent <CMD>          Agent command (claude, codex, amp, or custom)
  -h, --help             Show help
  -V, --version          Show version

Examples:
  ralph                  Run in current directory with defaults
  ralph 20               Run with 20 max iterations
  ralph -d ./my-project  Run in specified directory
  ralph init             Create prd.json template
  ralph --agent amp      Run with Amp CLI
```

## Codex + local models (Ollama)

If you want Ralph to use Codex with local models:

1) Start Ollama (for example, `cd ~/off-quant && tilt up`)
2) Export Codex settings:

```bash
export CODEX_OSS=1
export CODEX_OSS_PROVIDER=ollama
export CODEX_OSS_MODEL=local/qwen2.5-coder-7b-q4km
```

3) Run Ralph with Codex:

```bash
ralph --agent codex
```

## Workflow

### 1. Initialize your project

```bash
cd your-project
ralph --init
```

This creates a `prd.json` template.

### 2. Define your tasks

Edit `prd.json` with your user stories. Each story should be small enough to complete in one iteration. See `prd.json.example` for the format.

You can also use Claude to help create PRDs:

```bash
# Use the PRD template
cat $(ralph --home)/skills/prd/SKILL.md | claude --print "Create a PRD for [feature]"

# Then convert to ralph format
cat $(ralph --home)/skills/ralph/SKILL.md | claude --print "Convert this PRD to prd.json"
```

### 3. Run Ralph

```bash
ralph              # Run with defaults (10 iterations)
ralph 20           # Run with 20 max iterations
ralph -d ./other   # Run in different directory
ralph --parallel --parallel-queue-capacity 64 --parallel-queue-policy drop_oldest
```

Autonomous mode (skip checkpoint prompts and auto-resume when possible):

```bash
RALPH_AUTONOMOUS=1 ralph
RALPH_AUTONOMOUS=1 ralph --parallel
```

`--agent codex` also enables autonomous mode automatically.

To allow Codex to bypass approvals and sandboxing, set:

```bash
RALPH_CODEX_DANGEROUS=1 ralph --agent codex
```

Codex runs with `--json` output enabled for non-interactive execution.

Backpressure controls:

```bash
# MCP server queue size (default 32)
RALPH_QUEUE_SIZE=64 ralph mcp-server

# Parallel runner queue controls
ralph --parallel \
  --parallel-queue-capacity 64 \
  --parallel-queue-policy block

# Environment overrides for parallel queue
RALPH_PARALLEL_QUEUE_CAPACITY=64 \
RALPH_PARALLEL_QUEUE_POLICY=drop_oldest \
ralph --parallel
```

MCP queue status:

```bash
# Query the MCP tool for queue depth and ETA heuristics
get_queue_status
```

Ralph will:
1. Create a feature branch (from PRD `branchName`)
2. Pick the highest priority story where `passes: false`
3. Implement that single story
4. Run quality checks (typecheck, tests)
5. Commit if checks pass
6. Update `prd.json` to mark story as `passes: true`
7. Append learnings to `progress.txt`
8. Repeat until all stories pass or max iterations reached

## Key Files

| File | Purpose |
|------|---------|
| `bin/ralph` | Global CLI binary |
| `install.sh` | Installer script |
| `prompt.md` | Instructions given to each Claude Code instance |
| `prd.json.example` | Example PRD format for reference |
| `skills/prd/` | Prompt template for generating PRDs |
| `skills/ralph/` | Prompt template for converting PRDs to JSON |
| `flowchart/` | Interactive visualization of how Ralph works |

**In your project directory:**

| File | Purpose |
|------|---------|
| `prd.json` | User stories with `passes` status (created by `ralph --init`) |
| `progress.txt` | Append-only learnings for future iterations |
| `archive/` | Previous run archives |

## Flowchart

[![Ralph Flowchart](ralph-flowchart.png)](https://kcirtapfromspace.github.io/ralph-machineo/)

**[View Interactive Flowchart](https://kcirtapfromspace.github.io/ralph-machineo/)** - Click through to see each step with animations.

The `flowchart/` directory contains the source code. To run locally:

```bash
cd flowchart
npm install
npm run dev
```

## Critical Concepts

### Each Iteration = Fresh Context

Each iteration spawns a **new Claude Code instance** with clean context. The only memory between iterations is:
- Git history (commits from previous iterations)
- `progress.txt` (learnings and context)
- `prd.json` (which stories are done)

### Small Tasks

Each PRD item should be small enough to complete in one context window. If a task is too big, the LLM runs out of context before finishing and produces poor code.

Right-sized stories:
- Add a database column and migration
- Add a UI component to an existing page
- Update a server action with new logic
- Add a filter dropdown to a list

Too big (split these):
- "Build the entire dashboard"
- "Add authentication"
- "Refactor the API"

### AGENTS.md Updates Are Critical

After each iteration, Ralph updates the relevant `AGENTS.md` files with learnings. This is key because Claude Code automatically reads these files, so future iterations (and future human developers) benefit from discovered patterns, gotchas, and conventions.

Examples of what to add to AGENTS.md:
- Patterns discovered ("this codebase uses X for Y")
- Gotchas ("do not forget to update Z when changing W")
- Useful context ("the settings panel is in component X")

### Feedback Loops

Ralph only works if there are feedback loops:
- Typecheck catches type errors
- Tests verify behavior
- CI must stay green (broken code compounds across iterations)

### Browser Verification for UI Stories

Frontend stories must include "Verify in browser" in acceptance criteria. Ralph will navigate to the page, interact with the UI, and confirm changes work.

### Stop Condition

When all stories have `passes: true`, Ralph outputs `<promise>COMPLETE</promise>` and the loop exits.

## Debugging

Check current state:

```bash
# See which stories are done
cat prd.json | jq '.userStories[] | {id, title, passes}'

# See learnings from previous iterations
cat progress.txt

# Check git history
git log --oneline -10
```

## Customizing prompt.md

Edit `prompt.md` to customize Ralph's behavior for your project:
- Add project-specific quality check commands
- Include codebase conventions
- Add common gotchas for your stack

## Parallel Execution

Ralph can execute independent stories in parallel to speed up development. Stories that don't depend on each other run concurrently, while dependencies are respected.

### Enabling Parallel Mode

```bash
ralph --parallel                    # Enable parallel execution
ralph --parallel --max-concurrency 5  # Run up to 5 stories at once
```

### CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--parallel` | `false` | Enable parallel story execution |
| `--max-concurrency` | `3` | Maximum concurrent stories (0 = unlimited) |

### PRD Fields for Parallel Execution

Add these optional fields to your user stories in `prd.json`:

| Field | Type | Description |
|-------|------|-------------|
| `dependsOn` | `string[]` | Story IDs that must complete before this story starts |
| `targetFiles` | `string[]` | File paths/patterns this story will modify |

**How they work:**
- `dependsOn`: Explicit dependencies. Story won't start until all listed stories pass.
- `targetFiles`: Used for automatic conflict detection. Stories with overlapping files run sequentially to prevent merge conflicts.

### Example PRD with Dependencies

```json
{
  "branchName": "feature/user-auth",
  "userStories": [
    {
      "id": "US-001",
      "title": "Add user model",
      "targetFiles": ["src/models/user.rs", "src/models/mod.rs"],
      "passes": false
    },
    {
      "id": "US-002",
      "title": "Add auth middleware",
      "dependsOn": ["US-001"],
      "targetFiles": ["src/middleware/auth.rs"],
      "passes": false
    },
    {
      "id": "US-003",
      "title": "Add login endpoint",
      "dependsOn": ["US-001", "US-002"],
      "targetFiles": ["src/routes/auth.rs"],
      "passes": false
    },
    {
      "id": "US-004",
      "title": "Add user settings page",
      "dependsOn": ["US-001"],
      "targetFiles": ["src/pages/settings.rs"],
      "passes": false
    }
  ]
}
```

In this example:
- **US-001** runs first (no dependencies)
- **US-002** and **US-004** can run in parallel after US-001 completes (different target files)
- **US-003** waits for both US-001 and US-002

### Automatic Dependency Inference

When `targetFiles` patterns overlap between stories, Ralph automatically infers dependencies based on priority. Higher-priority stories (lower priority number) run first.

For example, if two stories both target `src/**/*.rs`, the higher-priority story becomes a dependency of the lower-priority one.

### Conflict Handling

Ralph prevents conflicts through:

1. **Pre-execution checks**: Stories with overlapping `targetFiles` don't run simultaneously
2. **File locking**: Each story locks its target files during execution
3. **Git mutex**: Git operations are serialized to prevent repository corruption
4. **Post-batch reconciliation**: After each parallel batch, Ralph verifies the codebase compiles and has no merge conflicts

If conflicts are detected, affected stories automatically retry sequentially.

## Archiving

Ralph automatically archives previous runs when you start a new feature (different `branchName`). Archives are saved to `archive/YYYY-MM-DD-feature-name/`.

## Docker Deployment

Ralph can also run as an MCP server in Docker for use with Claude Desktop:

| File | Purpose |
|------|---------|
| `Dockerfile` | Multi-stage build for Ralph MCP server |
| `docker-compose.yml` | Local development and testing |
| `.dockerignore` | Build context optimization |
| `examples/` | MCP toolkit configuration examples |
| `docs/guides/docker-mcp-setup.md` | Docker MCP setup guide |

```bash
# Build locally
docker build -t ralph-mcp .

# Run with docker-compose
docker compose up --build
```

Images are published to `ghcr.io/kcirtapfromspace/ralph-machineo` on every push to main.

## References

- [Geoffrey Huntley's Ralph article](https://ghuntley.com/ralph/)
- [Claude Code documentation](https://docs.anthropic.com/en/docs/claude-code)
- [Docker MCP Setup Guide](docs/guides/docker-mcp-setup.md)
