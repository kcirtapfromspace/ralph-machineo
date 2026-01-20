# PRD: Codex + Local Models Support

Version: 1.0
Status: Draft

## Purpose
Enable Ralph to run Codex CLI as an agent and use local OSS models (via Ollama), matching the local model setup in `~/off-quant`.

## Requirements
- Allow agent selection for `claude`, `codex`, `amp`, or custom commands
- Codex invocation supports OSS provider flags and local Ollama models
- Documentation explains how to run with local models and Ollama from `~/off-quant`
- Automated tests cover Codex invocation logic

## Acceptance Criteria
- CLI exposes `--agent` and passes through to execution layer
- Agent auto-detection prefers `claude`, then `codex`, then `amp`
- Codex invocation supports environment overrides:
  - `CODEX_OSS=1` to enable OSS provider
  - `CODEX_OSS_PROVIDER` (default `ollama`)
  - `CODEX_OSS_MODEL` for model tag
  - `CODEX_MODEL` for non-OSS model override
- README includes a Codex + Ollama setup section
- Tests validate Codex invocation and detection updates

## Out of Scope
- Changing Codex CLI behavior itself
- Installing Ollama or Codex for the user

