# codex-source

Implements fixture-gated Codex event source adapters. The initial adapter
accepts the exact Codex CLI 0.144.5 `Stop` hook shape verified in Stage 01,
hashes the source session ID, and emits fixed private task-completion text.

Raw working directories, transcript paths, model names, assistant messages,
prompts, responses, environment values, and unknown fields never enter the
canonical event. Unsupported versions and interfaces fail closed.
